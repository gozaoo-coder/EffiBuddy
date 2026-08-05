<script setup lang="ts">
/**
 * AsrStreamTab —— ASR 流式录入视图
 *
 * 全流程管线（与用户动画规则一致）：
 *   idle →（点击录音）→ recording →（再次点击 / 自动完成）→ finishing → done
 *                                ↘（错误）→ error
 *   任意阶段中断（切走页签 / 关闭）：onBeforeUnmount 触发 cancel，释放麦克风 + 取消会话
 *
 * 数据流：
 *   useAudioRecorder.onChunk(b64) → useAsr.pushAudio(sessionId, b64)
 *   asr-stream-chunk 事件 → 实时拼接 transcript（is_final=false 中间灰色，true 转黑）
 *   asr-session-status 事件 → completed/failed 时停止录音 + 显示结果
 *
 * 页签可关闭性：录音中通过 useTabs.updateTab 设置 closable=false，结束/取消后恢复。
 */
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { animate } from 'animejs'
import Icon from '../Icon.vue'
import { Button, Dropdown, useToast } from '../basic'
import { useAsr } from '../../composables/useAsr'
import { useAudioRecorder } from '../../composables/useAudioRecorder'
import { useTabs } from '../../composables/useTabs'
import type {
  AsrFinishResult,
  AsrSessionStatusPayload,
  AsrStreamChunkPayload,
  TabItem,
} from '../../types'

defineOptions({ name: 'AsrStreamTab' })

const props = defineProps<{
  tab: TabItem
}>()

const emit = defineEmits<{
  (e: 'update:status', status: TabItem['status']): void
}>()

const { toast } = useToast()
const {
  startStreaming,
  pushAudio,
  finishStreaming,
  cancelStreaming,
  onStreamChunk,
  onSessionStatus,
} = useAsr()
const { updateTab } = useTabs()
const recorder = useAudioRecorder()

// ============= 状态机 =============
type Phase = 'idle' | 'recording' | 'finishing' | 'done' | 'error'
const phase = ref<Phase>('idle')
const sessionId = ref<string | null>(null)
const finalText = ref('')
const interimText = ref('')
const errorMessage = ref('')
const finishResult = ref<AsrFinishResult | null>(null)

// 语言选项
const langOptions = [
  { value: 'zh-CN', label: '中文（普通话）' },
  { value: 'en-US', label: '英语（美国）' },
  { value: 'en-GB', label: '英语（英国）' },
  { value: 'ja-JP', label: '日语' },
  { value: 'ko-KR', label: '韩语' },
  { value: '', label: '自动检测' },
]
const lang = ref('zh-CN')

const elapsedSec = recorder.elapsedSec

const statusText = computed(() => {
  switch (phase.value) {
    case 'idle':
      return '点击开始录音'
    case 'recording':
      return '录音中…'
    case 'finishing':
      return '处理中…'
    case 'done':
      return '转写完成'
    case 'error':
      return '录音出错'
  }
})

const displayTranscript = computed(() => {
  const f = finalText.value
  const i = interimText.value
  if (i) return f + i
  return f
})

// ============= 动画 =============
const rippleRef = ref<HTMLElement | null>(null)
const transcriptRef = ref<HTMLElement | null>(null)

/** 录音按钮按下：红色圆扩散涟漪（一次性，非循环） */
function playStartRipple(): void {
  const el = rippleRef.value
  if (!el) return
  el.style.opacity = '0.6'
  animate(el, {
    scale: [1, 1.8],
    opacity: [0.6, 0],
    duration: 700,
    ease: 'out(3)',
    onComplete: () => {
      el.style.opacity = ''
    },
  })
}

/** 中间结果 → 最终结果时，transcript 区域背景闪一下高亮 */
let flashAnim: ReturnType<typeof animate> | null = null
function flashTranscript(): void {
  const el = transcriptRef.value
  if (!el) return
  if (flashAnim) flashAnim.pause()
  flashAnim = animate(el, {
    backgroundColor: [
      'rgba(74, 126, 255, 0)',
      'rgba(74, 126, 255, 0.14)',
      'rgba(74, 126, 255, 0)',
    ],
    duration: 600,
    ease: 'out(2)',
  })
}

// 监听 finalText 变化：新 token 到达时高亮 + 滚到底部
watch(finalText, async () => {
  await nextTick()
  flashTranscript()
  scrollTranscriptToBottom()
})

watch(interimText, async () => {
  await nextTick()
  scrollTranscriptToBottom()
})

function scrollTranscriptToBottom(): void {
  const el = transcriptRef.value
  if (el) el.scrollTop = el.scrollHeight
}

// ============= 事件订阅清理 =============
let unsubChunk: (() => void) | null = null
let unsubStatus: (() => void) | null = null

function bindEvents(): void {
  unsubChunk = onStreamChunk((p: AsrStreamChunkPayload) => {
    if (sessionId.value && p.session_id !== sessionId.value) return
    if (p.is_final) {
      // 确定结果：追加到 finalText，清空中间结果
      if (p.text) finalText.value += p.text
      interimText.value = ''
    } else {
      // 中间结果：替换
      interimText.value = p.text
    }
  })

  unsubStatus = onSessionStatus((p: AsrSessionStatusPayload) => {
    if (sessionId.value && p.session_id !== sessionId.value) return
    if (p.status === 'failed') {
      errorMessage.value = p.error ?? '转写失败'
      handleError(new Error(errorMessage.value))
    } else if (p.status === 'completed') {
      // 后端已结束，自动走 finish 流程（若尚未触发）
      void autoFinish()
    }
  })
}

// ============= 录音流程 =============
let finishing = false

async function startRecording(): Promise<void> {
  if (phase.value === 'recording' || phase.value === 'finishing') return
  errorMessage.value = ''
  finishResult.value = null
  finalText.value = ''
  interimText.value = ''

  // 1. 先建 ASR 流式通道（拿 session_id），再开麦克风
  let sid: string
  try {
    sid = await startStreaming(lang.value || null)
  } catch (e) {
    toast({ content: `启动转写失败：${e}`, type: 'error' })
    phase.value = 'error'
    emit('update:status', 'error')
    return
  }
  sessionId.value = sid

  // 2. 启动麦克风采集；onChunk 推送到后端
  try {
    await recorder.start(async (b64: string) => {
      if (!sessionId.value) return
      try {
        await pushAudio(sessionId.value, b64)
      } catch {
        /* 单帧推送失败不中断录音；后端会因静默超时自行处理 */
      }
    })
  } catch (e) {
    // 麦克风失败：取消刚建的 ASR 会话
    await cancelStreaming(sid).catch(() => {})
    sessionId.value = null
    const msg = e instanceof Error ? e.message : String(e)
    toast({ content: msg, type: 'error' })
    errorMessage.value = msg
    phase.value = 'error'
    emit('update:status', 'error')
    return
  }

  // 3. 进入录音态：锁页签 + 涟漪动画 + 状态上报
  phase.value = 'recording'
  emit('update:status', 'recording')
  updateTab(props.tab.id, { closable: false, status: 'recording' })
  playStartRipple()
}

/** 用户再次点击：正常停止 → finish */
async function stopRecording(): Promise<void> {
  if (finishing) return
  finishing = true
  phase.value = 'finishing'
  emit('update:status', 'loading')
  updateTab(props.tab.id, { status: 'loading' })

  // 1. 先停麦克风（flush 最后一帧）
  await recorder.stop().catch(() => {})

  // 2. 调 finish 拿完整 transcript + summary
  const sid = sessionId.value
  if (!sid) {
    finishing = false
    return
  }
  try {
    const result = await finishStreaming(sid)
    finishResult.value = result
    // 若后端有最终 transcript 且本地累积为空，用后端的
    if (result.transcript && !finalText.value) {
      finalText.value = result.transcript
    }
    phase.value = 'done'
    emit('update:status', 'active')
    updateTab(props.tab.id, { status: 'active', closable: true })
    toast({
      content: result.summary ? '转写完成，已生成摘要' : '转写完成',
      type: 'success',
    })
  } catch (e) {
    handleError(new Error(String(e)))
  } finally {
    finishing = false
    sessionId.value = null
  }
}

/** 后端 completed 事件触发的自动 finish（防止重复调用） */
let autoFinishing = false
async function autoFinish(): Promise<void> {
  if (autoFinishing || finishing) return
  if (phase.value !== 'recording') return
  autoFinishing = true
  try {
    await stopRecording()
  } finally {
    autoFinishing = false
  }
}

function handleError(err: Error): void {
  errorMessage.value = err.message
  phase.value = 'error'
  emit('update:status', 'error')
  updateTab(props.tab.id, { status: 'error', closable: true })
  // 清理麦克风
  void recorder.cancel().catch(() => {})
  sessionId.value = null
}

/** 取消录音（中断管线）：丢弃结果，立即释放 */
async function cancelRecording(): Promise<void> {
  await recorder.cancel().catch(() => {})
  const sid = sessionId.value
  if (sid) await cancelStreaming(sid).catch(() => {})
  sessionId.value = null
  finalText.value = ''
  interimText.value = ''
  phase.value = 'idle'
  emit('update:status', 'idle')
  updateTab(props.tab.id, { status: 'idle', closable: true })
}

/** 点击录音按钮：idle↔recording 切换，done/error 时重置 */
async function onRecordClick(): Promise<void> {
  if (phase.value === 'recording') {
    await stopRecording()
  } else if (phase.value === 'idle' || phase.value === 'done' || phase.value === 'error') {
    await startRecording()
  }
  // finishing 阶段忽略点击
}

// ============= 跳转历史 =============
function gotoHistory(): void {
  useTabs().openTab({
    id: 'asr-history',
    kind: 'asr-history',
    title: 'ASR 历史',
    closable: true,
    instanceKey: '',
  })
}

// ============= 时间格式化 =============
const elapsedDisplay = computed(() => {
  const s = elapsedSec.value
  const mm = Math.floor(s / 60)
    .toString()
    .padStart(2, '0')
  const ss = (s % 60).toString().padStart(2, '0')
  return `${mm}:${ss}`
})

// ============= 生命周期 =============
function init(): void {
  emit('update:status', 'idle')
  bindEvents()
}

function destroy(): void {
  // 中断管线：清理麦克风 + 会话（切走页签时调用）
  void (async () => {
    await recorder.cancel().catch(() => {})
    const sid = sessionId.value
    if (sid) await cancelStreaming(sid).catch(() => {})
    sessionId.value = null
    if (phase.value === 'recording' || phase.value === 'finishing') {
      phase.value = 'idle'
      updateTab(props.tab.id, { status: 'idle', closable: true })
    }
  })()
}

onMounted(() => {
  init()
})

onBeforeUnmount(() => {
  destroy()
  unsubChunk?.()
  unsubStatus?.()
})

defineExpose({ init, destroy })
</script>

<template>
  <div class="asr-tab asr-stream-tab">
    <div class="stream-hero">
      <!-- 录音按钮：脉冲 + 涟漪 -->
      <div class="record-stage">
        <span ref="rippleRef" class="record-ripple" />
        <button
          type="button"
          class="record-btn"
          :class="{
            recording: phase === 'recording',
            finishing: phase === 'finishing',
            done: phase === 'done',
            error: phase === 'error',
          }"
          :disabled="phase === 'finishing'"
          :aria-label="statusText"
          @click="onRecordClick"
        >
          <Icon
            :name="
              phase === 'recording'
                ? 'close'
                : phase === 'finishing'
                  ? 'loader'
                  : 'mic'
            "
            :size="32"
          />
          <span v-if="phase === 'recording'" class="pulse-ring" />
        </button>
      </div>

      <!-- 状态文字：cross-fade -->
      <Transition name="status-fade" mode="out-in">
        <p :key="statusText" class="status-text">{{ statusText }}</p>
      </Transition>

      <p v-if="phase === 'done' && finishResult" class="result-meta">
        <Icon name="check" :size="14" /> 转写已归档至历史记录
      </p>
      <p v-else-if="phase === 'error'" class="result-meta error-meta">
        <Icon name="warning" :size="14" /> {{ errorMessage }}
      </p>
    </div>

    <!-- 实时转写区 -->
    <div ref="transcriptRef" class="transcript-area">
      <template v-if="displayTranscript || phase === 'recording' || phase === 'finishing'">
        <span class="transcript-final">{{ finalText }}</span><span
          v-if="interimText"
          class="transcript-interim"
        >{{ interimText }}</span>
        <span v-if="phase === 'recording' && !interimText" class="transcript-cursor" />
        <p v-if="!displayTranscript && phase !== 'finishing'" class="transcript-hint">
          说话内容将实时显示在这里…
        </p>
      </template>

      <!-- 完成：展示 summary -->
      <div v-if="phase === 'done' && finishResult?.summary" class="summary-block">
        <div class="summary-head">
          <Icon name="sparkles" :size="16" />
          <span>AI 摘要</span>
        </div>
        <p class="summary-text">{{ finishResult.summary }}</p>
      </div>
    </div>

    <!-- 底部控制栏 -->
    <div class="stream-footer">
      <div class="footer-left">
        <Dropdown
          :options="langOptions"
          :model-value="lang"
          placeholder="语言"
          size="sm"
          width="auto"
          :disabled="phase === 'recording' || phase === 'finishing'"
          @update:model-value="lang = String($event)"
        />
      </div>

      <div class="footer-center">
        <span v-if="phase === 'recording' || phase === 'finishing'" class="timer">
          <span class="timer-dot" />{{ elapsedDisplay }}
        </span>
      </div>

      <div class="footer-right">
        <Button
          v-if="phase === 'recording'"
          variant="normal"
          size="sm"
          @click="cancelRecording"
        >
          取消
        </Button>
        <Button
          v-if="phase === 'done'"
          variant="primary"
          size="sm"
          @click="gotoHistory"
        >
          查看历史
        </Button>
        <Button
          v-if="phase === 'error'"
          variant="normal"
          size="sm"
          @click="startRecording"
        >
          重试
        </Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.asr-tab {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--bg);
  padding: var(--space-6) var(--space-8) var(--space-4);
  gap: var(--space-5);
}

/* ---------- 顶部录音舞台 ---------- */
.stream-hero {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-6) 0 var(--space-2);
  flex-shrink: 0;
}

.record-stage {
  position: relative;
  width: 96px;
  height: 96px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.record-ripple {
  position: absolute;
  inset: 0;
  border-radius: var(--radius-full);
  border: 2px solid var(--danger);
  opacity: 0;
  pointer-events: none;
}

.record-btn {
  position: relative;
  width: 76px;
  height: 76px;
  border-radius: var(--radius-full);
  border: none;
  background: var(--card);
  color: var(--text);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  box-shadow: var(--shadow);
  transition: transform var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.record-btn:not(:disabled):hover {
  transform: scale(1.05);
}

.record-btn:not(:disabled):active {
  transform: scale(0.95);
}

.record-btn.recording {
  background: var(--danger);
  color: #fff;
}

.record-btn.finishing {
  background: var(--card-2);
  color: var(--muted);
  cursor: progress;
}

.record-btn.done {
  background: rgba(62, 207, 142, 0.14);
  color: var(--success);
}

.record-btn.error {
  background: rgba(255, 92, 92, 0.12);
  color: var(--danger);
}

/* 录音中持续脉冲（CSS 动画，不占用主线程） */
.pulse-ring {
  position: absolute;
  inset: -6px;
  border-radius: var(--radius-full);
  border: 2px solid var(--danger);
  opacity: 0;
  animation: asr-pulse 1.8s var(--ease-standard) infinite;
}

@keyframes asr-pulse {
  0% {
    transform: scale(0.92);
    opacity: 0.55;
  }
  70% {
    transform: scale(1.3);
    opacity: 0;
  }
  100% {
    transform: scale(1.3);
    opacity: 0;
  }
}

.status-text {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 500;
  color: var(--text);
}

/* 状态文字 cross-fade */
.status-fade-enter-active,
.status-fade-leave-active {
  transition: opacity var(--duration-base) var(--ease-standard),
    transform var(--duration-base) var(--ease-standard);
}
.status-fade-enter-from {
  opacity: 0;
  transform: translateY(6px);
}
.status-fade-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}

.result-meta {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--success);
}

.error-meta {
  color: var(--danger);
}

/* ---------- 转写区 ---------- */
.transcript-area {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-5) var(--space-6);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  font-size: var(--fs-md);
  line-height: 1.75;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  transition: background-color var(--duration-base) var(--ease-standard);
}

.transcript-final {
  color: var(--text);
}

.transcript-interim {
  color: var(--muted);
}

.transcript-cursor {
  display: inline-block;
  width: 2px;
  height: 1.1em;
  margin-left: 2px;
  vertical-align: text-bottom;
  background: var(--primary);
  animation: blink 1s steps(2, start) infinite;
}

@keyframes blink {
  to {
    opacity: 0;
  }
}

.transcript-hint {
  color: var(--muted);
  font-size: var(--fs-base);
  margin: 0;
}

.summary-block {
  margin-top: var(--space-5);
  padding-top: var(--space-4);
  border-top: 1px dashed var(--border);
}

.summary-head {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--primary);
  font-size: var(--fs-sm);
  font-weight: 600;
  margin-bottom: var(--space-2);
}

.summary-text {
  margin: 0;
  font-size: var(--fs-base);
  color: var(--text);
  line-height: 1.7;
  white-space: pre-wrap;
}

/* ---------- 底部栏 ---------- */
.stream-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-2);
  border-top: 1px solid var(--border);
  flex-shrink: 0;
}

.footer-left,
.footer-right {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.footer-center {
  flex: 1;
  display: flex;
  justify-content: center;
}

.lang-trigger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--card);
  color: var(--text);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.timer {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: 'SFMono-Regular', Consolas, monospace;
  font-size: var(--fs-md);
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

.timer-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--danger);
  animation: timer-blink 1s ease-in-out infinite;
}

@keyframes timer-blink {
  50% {
    opacity: 0.3;
  }
}
</style>
