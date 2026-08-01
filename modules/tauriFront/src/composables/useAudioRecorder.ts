/**
 * useAudioRecorder —— 麦克风 PCM 采集组合式函数
 *
 * 采集规格：16kHz / 16bit / mono PCM（与火山引擎 / Qwen ASR 流式接口对齐）。
 * 每 ~200ms（3200 样本 × 2 字节 = 6400 字节）回调一次 base64 编码的 PCM 帧。
 *
 * 实现选型：使用 ScriptProcessorNode 而非 AudioWorklet。
 * 原因：Tauri 2 webview 对 AudioWorklet 的 CSP / 跨上下文支持参差，
 * ScriptProcessorNode 兼容性最好（虽已 deprecated，但在主线程采集 16k 帧足够轻量）。
 *
 * 降采样：系统 AudioContext 通常是 44.1k/48k，需降到 16k。
 * 采用「整数倍抽样 + 余数累积」：当源/目标比为非整数（如 44100/16000=2.756）时，
 * 用线性插值而非简单丢弃，避免频谱混叠失真。
 *
 * 生命周期管线（与用户动画规则呼应）：
 *   start() → 申请权限 / 建 AudioContext / 连 ScriptProcessor → 持续回调 chunk
 *   stop()  → 停止采集 → flush 残留缓冲 → 释放 mic track → 关闭 AudioContext
 *   cancel()→ 立即停止采集 → 丢弃缓冲 → 释放资源（不回调残留）
 *   错误：权限拒绝 / 设备不可用 → 抛出携带原因的 Error，调用方决定 toast
 */
import { ref, readonly, type DeepReadonly, type Ref } from 'vue'

export interface AudioRecorderError extends Error {
  /** 'permission_denied' | 'no_device' | 'context_unavailable' | 'busy' */
  code: string
}

const TARGET_SAMPLE_RATE = 16000
/** 每帧样本数（200ms @ 16kHz） */
const CHUNK_SAMPLES = 3200
/** PCM 16bit = 2 字节/样本 */
const BYTES_PER_SAMPLE = 2

type ChunkCb = (base64: string) => void
type StateCb = (recording: boolean) => void

function makeError(code: string, message: string): AudioRecorderError {
  const err = new Error(message) as AudioRecorderError
  err.code = code
  return err
}

export interface UseAudioRecorderReturn {
  isRecording: DeepReadonly<Ref<boolean>>
  /** 当前采集时长（秒），录音中实时更新 */
  elapsedSec: DeepReadonly<Ref<number>>
  /** 启动采集；onChunk 在每帧就绪时回调 */
  start: (onChunk: ChunkCb) => Promise<void>
  /** 正常停止：flush 残留缓冲后释放资源 */
  stop: () => Promise<void>
  /** 取消：丢弃缓冲并立即释放资源 */
  cancel: () => Promise<void>
  /** 订阅状态变化（recording true/false） */
  onStateChange: (cb: StateCb) => () => void
}

export function useAudioRecorder(): UseAudioRecorderReturn {
  const isRecording = ref(false)
  const elapsedSec = ref(0)

  // 采集句柄
  let audioCtx: AudioContext | null = null
  let mediaStream: MediaStream | null = null
  let sourceNode: MediaStreamAudioSourceNode | null = null
  let processorNode: ScriptProcessorNode | null = null

  // 降采样缓冲：累积 16k Float32 样本到 CHUNK_SAMPLES 后发射一帧
  let resampleBuffer: Float32Array = new Float32Array(CHUNK_SAMPLES)
  let resampleWrite = 0
  // 余数累积：处理源/目标比非整数时的相位对齐
  let fracAccum = 0

  // 计时器
  let elapsedTimer: number | null = null
  let startTime = 0

  // 回调
  let chunkCb: ChunkCb | null = null
  const stateCbs = new Set<StateCb>()

  function notifyState(recording: boolean): void {
    isRecording.value = recording
    stateCbs.forEach((cb) => {
      try {
        cb(recording)
      } catch {
        /* ignore */
      }
    })
  }

  function onStateChange(cb: StateCb): () => void {
    stateCbs.add(cb)
    return () => stateCbs.delete(cb)
  }

  /** Float32 样本 → 16bit PCM little-endian 字节数组 */
  function floatTo16Pcm(samples: Float32Array): Uint8Array {
    const out = new Uint8Array(samples.length * BYTES_PER_SAMPLE)
    const view = new DataView(out.buffer)
    for (let i = 0; i < samples.length; i++) {
      // 钳制到 [-1, 1] 后转 Int16
      const s = Math.max(-1, Math.min(1, samples[i]))
      const int16 = s < 0 ? s * 0x8000 : s * 0x7fff
      view.setInt16(i * 2, int16, true)
    }
    return out
  }

  /** Uint8Array → base64 字符串（分块处理避免栈溢出） */
  function toBase64(bytes: Uint8Array): string {
    let binary = ''
    const chunk = 0x8000
    for (let i = 0; i < bytes.length; i += chunk) {
      const slice = bytes.subarray(i, Math.min(i + chunk, bytes.length))
      binary += String.fromCharCode.apply(null, Array.from(slice))
    }
    return btoa(binary)
  }

  /** 发射一帧：把 resampleBuffer 前 CHUNK_SAMPLES 转 PCM + base64 */
  function emitChunk(): void {
    if (!chunkCb) return
    const frame = resampleBuffer.subarray(0, CHUNK_SAMPLES)
    const pcm = floatTo16Pcm(frame)
    chunkCb(toBase64(pcm))
  }

  /** 降采样（线性插值）并累积到 resampleBuffer，满 CHUNK_SAMPLES 即发射 */
  function processInput(input: Float32Array): void {
    const srcLen = input.length
    if (srcLen === 0) return
    const srcRate = audioCtx?.sampleRate ?? 48000
    const ratio = srcRate / TARGET_SAMPLE_RATE

    let srcIdx = 0
    // 第一次进入时校正余数相位
    let pos = fracAccum

    while (srcIdx < srcLen) {
      // 当前目标样本对应的源位置
      const i0 = Math.floor(pos)
      const i1 = Math.min(i0 + 1, srcLen - 1)
      const frac = pos - i0
      if (i0 < srcLen) {
        const sample = input[i0] * (1 - frac) + input[i1] * frac
        resampleBuffer[resampleWrite++] = sample
        if (resampleWrite >= CHUNK_SAMPLES) {
          emitChunk()
          // 环形复用：写指针归零，下一帧从头覆盖
          resampleWrite = 0
        }
      }
      pos += ratio
      srcIdx = Math.floor(pos)
    }
    // 保留小数相位，下一轮对齐
    fracAccum = pos - srcIdx
  }

  async function start(onChunk: ChunkCb): Promise<void> {
    if (isRecording.value) {
      throw makeError('busy', '已在录音中，请先停止当前录制')
    }
    chunkCb = onChunk
    resampleWrite = 0
    fracAccum = 0

    if (typeof navigator === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
      throw makeError('context_unavailable', '当前环境不支持麦克风采集')
    }

    let stream: MediaStream
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
        video: false,
      })
    } catch (e) {
      const err = e as DOMException
      if (err?.name === 'NotAllowedError' || err?.name === 'SecurityError') {
        throw makeError('permission_denied', '麦克风权限被拒绝，请在系统设置中允许访问')
      }
      if (err?.name === 'NotFoundError' || err?.name === 'OverconstrainedError') {
        throw makeError('no_device', '未找到可用的麦克风设备')
      }
      throw makeError('context_unavailable', `麦克风启动失败：${err?.message ?? e}`)
    }

    mediaStream = stream
    // AudioContext：采样率由系统决定，processInput 内做降采样
    const Ctor: typeof AudioContext =
      window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
    audioCtx = new Ctor()
    if (audioCtx.state === 'suspended') {
      await audioCtx.resume().catch(() => {})
    }
    sourceNode = audioCtx.createMediaStreamSource(stream)
    // bufferSize=4096：在 48k 下约 85ms 触发一次，足够实时
    processorNode = audioCtx.createScriptProcessor(4096, 1, 1)
    processorNode.onaudioprocess = (ev: AudioProcessingEvent) => {
      if (!isRecording.value) return
      const input = ev.inputBuffer.getChannelData(0)
      processInput(input)
    }
    sourceNode.connect(processorNode)
    // ScriptProcessorNode 必须连接 destination 才会触发 onaudioprocess
    // （即使不输出声音也需连接；用零增益避免回声）
    const muteGain = audioCtx.createGain()
    muteGain.gain.value = 0
    processorNode.connect(muteGain)
    muteGain.connect(audioCtx.destination)

    startTime = Date.now()
    elapsedSec.value = 0
    elapsedTimer = window.setInterval(() => {
      elapsedSec.value = Math.floor((Date.now() - startTime) / 1000)
    }, 250)

    notifyState(true)
  }

  /** 释放底层资源（mic / context），stopFlush=true 时先 flush 残留缓冲 */
  async function teardown(stopFlush: boolean): Promise<void> {
    if (elapsedTimer !== null) {
      window.clearInterval(elapsedTimer)
      elapsedTimer = null
    }
    if (stopFlush && resampleWrite > 0 && chunkCb) {
      // 残留不足一帧：补零到 CHUNK_SAMPLES 后发射最后一帧
      const last = new Float32Array(CHUNK_SAMPLES)
      last.set(resampleBuffer.subarray(0, resampleWrite))
      const pcm = floatTo16Pcm(last)
      chunkCb(toBase64(pcm))
    }
    // 重置缓冲（无论 flush 与否）
    resampleWrite = 0
    fracAccum = 0

    notifyState(false)

    if (processorNode) {
      processorNode.onaudioprocess = null
      try {
        processorNode.disconnect()
      } catch {
        /* ignore */
      }
      processorNode = null
    }
    if (sourceNode) {
      try {
        sourceNode.disconnect()
      } catch {
        /* ignore */
      }
      sourceNode = null
    }
    if (mediaStream) {
      mediaStream.getTracks().forEach((t) => t.stop())
      mediaStream = null
    }
    if (audioCtx) {
      try {
        await audioCtx.close()
      } catch {
        /* ignore */
      }
      audioCtx = null
    }
    chunkCb = null
  }

  async function stop(): Promise<void> {
    if (!isRecording.value) return
    await teardown(true)
  }

  async function cancel(): Promise<void> {
    if (!isRecording.value) return
    await teardown(false)
  }

  return {
    isRecording: readonly(isRecording),
    elapsedSec: readonly(elapsedSec),
    start,
    stop,
    cancel,
    onStateChange,
  }
}
