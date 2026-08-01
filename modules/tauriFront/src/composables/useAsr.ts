/**
 * useAsr —— ASR 语音转写业务逻辑组合式函数（全局单例）
 *
 * 设计要点（与 useTabs 一致的 module-level 单例模式）：
 * 1. 状态在 module-level 声明，所有调用 useAsr() 的组件共享同一份 records / sessions / config，
 *    天然实现跨组件通信（流式页签写入 / 历史页签读取 / 配置面板编辑均一致）。
 * 2. invoke 调用薄封装：命令名 snake_case，参数键 camelCase（Tauri 2 自动转 snake_case）。
 * 3. 事件订阅采用「注册回调」模式而非各自 listen：由 App.vue 在 setup 时调用 install() 一次，
 *    组件用 onStreamChunk / onSessionStatus / onUploadProgress / onRecordUpdated 订阅。
 *    这样避免每个组件各自 listen 导致的重复订阅与泄漏；组件卸载时调用返回的取消函数即可。
 * 4. asr-record-updated 默认触发 listRecords 刷新（节流），组件可额外订阅做局部更新。
 */
import { ref, readonly, type Ref, type DeepReadonly } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  AsrConfig,
  AsrFinishResult,
  AsrRecord,
  AsrRecordPatch,
  AsrSessionInfo,
  AsrSummaryHit,
  AsrUploadProgressPayload,
  AsrRecordUpdatedPayload,
  AsrSessionStatusPayload,
  AsrStreamChunkPayload,
} from '../types'

// ============= module-level 单例状态 =============
const records = ref<AsrRecord[]>([])
const sessions = ref<AsrSessionInfo[]>([])
const config = ref<AsrConfig | null>(null)
const loading = ref(false)

// ============= 事件回调注册表 =============
// 用 Set 存放回调，卸载时 O(1) 删除；回调可返回 false 阻止默认行为（如自管理刷新）
type StreamChunkCb = (p: AsrStreamChunkPayload) => void
type SessionStatusCb = (p: AsrSessionStatusPayload) => void
type UploadProgressCb = (p: AsrUploadProgressPayload) => void
type RecordUpdatedCb = (p: AsrRecordUpdatedPayload) => void

const chunkCbs = new Set<StreamChunkCb>()
const statusCbs = new Set<SessionStatusCb>()
const uploadCbs = new Set<UploadProgressCb>()
const recordUpdatedCbs = new Set<RecordUpdatedCb>()

// listRecords 刷新节流：asr-record-updated 高频触发时合并为一次刷新
let refreshTimer: number | null = null
let installed = false
let unlistens: UnlistenFn[] = []

function scheduleRefresh(): void {
  if (refreshTimer !== null) return
  refreshTimer = window.setTimeout(async () => {
    refreshTimer = null
    try {
      await listRecords()
    } catch {
      /* 静默：刷新失败不阻塞事件流 */
    }
  }, 240)
}

// ============= 命令封装（薄 invoke） =============

/** 列出全部 ASR 记录元数据（不含 transcript），写入 records 响应式状态 */
async function listRecords(): Promise<AsrRecord[]> {
  loading.value = true
  try {
    const list = await invoke<AsrRecord[]>('asr_list_records')
    records.value = list
    return list
  } finally {
    loading.value = false
  }
}

/** 获取单条记录（含完整 transcript），不写入全局状态 */
async function getRecord(recordId: string): Promise<AsrRecord | null> {
  return invoke<AsrRecord | null>('asr_get_record', { recordId })
}

/** 删除记录并从本地状态移除（不等 listRecords 刷新，UI 即时反馈） */
async function deleteRecord(recordId: string): Promise<void> {
  await invoke<null>('asr_delete_record', { recordId })
  records.value = records.value.filter((r) => r.id !== recordId)
}

/** 搜索记录（按关键词 / 来源 / 状态过滤），写入 records 响应式状态 */
async function searchRecords(opts: {
  keyword?: string | null
  limit?: number | null
  source?: string | null
  status?: string | null
}): Promise<AsrRecord[]> {
  const list = await invoke<AsrRecord[]>('asr_search_records', {
    keyword: opts.keyword ?? null,
    limit: opts.limit ?? null,
    source: opts.source ?? null,
    status: opts.status ?? null,
  })
  records.value = list
  return list
}

/** 摘要 RAG 检索（BM25 词法匹配） */
async function searchSummaries(
  keyword: string,
  limit: number | null = 10,
): Promise<AsrSummaryHit[]> {
  return invoke<AsrSummaryHit[]>('asr_search_summaries', {
    keyword,
    limit,
  })
}

/** 启动流式 ASR 会话，返回 session_id */
async function startStreaming(lang?: string | null): Promise<string> {
  return invoke<string>('asr_start_streaming', { lang: lang ?? null })
}

/** 推送一帧 base64 编码的 PCM 音频到活跃会话 */
async function pushAudio(sessionId: string, audioBase64: string): Promise<void> {
  await invoke<null>('asr_push_audio', {
    sessionId,
    audioBase64,
  })
}

/** 结束流式转写，返回完整 transcript + 摘要（若启用） */
async function finishStreaming(sessionId: string): Promise<AsrFinishResult> {
  return invoke<AsrFinishResult>('asr_finish_streaming', { sessionId })
}

/** 取消流式会话（幂等） */
async function cancelStreaming(sessionId: string): Promise<void> {
  await invoke<null>('asr_cancel_streaming', { sessionId })
}

/** 转写本地音频文件（一次性，非流式） */
async function transcribeFile(
  audioPath: string,
  lang?: string | null,
): Promise<AsrFinishResult> {
  return invoke<AsrFinishResult>('asr_transcribe_file', {
    audioPath,
    lang: lang ?? null,
  })
}

/** 对已转写记录生成结构化摘要，返回新摘要文本 */
async function generateSummary(recordId: string): Promise<string | null> {
  return invoke<string | null>('asr_generate_summary', { recordId })
}

/** 更新记录（标题 / 标签 / 摘要），返回更新后的记录并同步本地状态 */
async function updateRecord(
  recordId: string,
  patch: AsrRecordPatch,
): Promise<AsrRecord> {
  const updated = await invoke<AsrRecord>('asr_update_record', {
    recordId,
    title: patch.title ?? null,
    tags: patch.tags ?? null,
    summary: patch.summary ?? null,
  })
  // 就地替换对应项，保持引用稳定
  const idx = records.value.findIndex((r) => r.id === recordId)
  if (idx !== -1) {
    records.value.splice(idx, 1, updated)
  }
  return updated
}

/** 读取 ASR 配置快照，写入 config 响应式状态 */
async function loadConfig(): Promise<AsrConfig> {
  const cfg = await invoke<AsrConfig>('asr_get_config')
  config.value = cfg
  return cfg
}

/** 热更新 ASR 配置（provider 切换需重启生效） */
async function saveConfig(cfg: AsrConfig): Promise<void> {
  await invoke<null>('asr_update_config', { config: cfg })
  config.value = cfg
}

/** 列出活跃流式会话，写入 sessions 响应式状态 */
async function listSessions(): Promise<AsrSessionInfo[]> {
  const list = await invoke<AsrSessionInfo[]>('asr_list_sessions')
  sessions.value = list
  return list
}

// ============= 事件订阅 API =============
// 组件 setup 中调用，onUnmounted 时调用返回的取消函数即可。

function onStreamChunk(cb: StreamChunkCb): () => void {
  chunkCbs.add(cb)
  return () => chunkCbs.delete(cb)
}

function onSessionStatus(cb: SessionStatusCb): () => void {
  statusCbs.add(cb)
  return () => statusCbs.delete(cb)
}

function onUploadProgress(cb: UploadProgressCb): () => void {
  uploadCbs.add(cb)
  return () => uploadCbs.delete(cb)
}

function onRecordUpdated(cb: RecordUpdatedCb): () => void {
  recordUpdatedCbs.add(cb)
  return () => recordUpdatedCbs.delete(cb)
}

// ============= install：App.vue 在 onMounted 调用一次，绑定 4 个事件监听 =============

async function install(): Promise<() => void> {
  if (installed) {
    // 已安装：返回空卸载函数（避免重复监听）
    return () => {}
  }
  installed = true

  unlistens.push(
    await listen<AsrStreamChunkPayload>('asr-stream-chunk', (e) => {
      const p = e.payload
      chunkCbs.forEach((cb) => {
        try {
          cb(p)
        } catch {
          /* 单个回调异常不阻断其它订阅 */
        }
      })
    }),
  )

  unlistens.push(
    await listen<AsrSessionStatusPayload>('asr-session-status', (e) => {
      const p = e.payload
      statusCbs.forEach((cb) => {
        try {
          cb(p)
        } catch {
          /* ignore */
        }
      })
      // 会话结束（completed/failed/cancelled）后刷新活跃会话列表
      if (
        p.status === 'completed' ||
        p.status === 'failed' ||
        p.status === 'cancelled'
      ) {
        void listSessions()
      }
    }),
  )

  unlistens.push(
    await listen<AsrUploadProgressPayload>('asr-upload-progress', (e) => {
      const p = e.payload
      uploadCbs.forEach((cb) => {
        try {
          cb(p)
        } catch {
          /* ignore */
        }
      })
      // 上传转写完成时刷新记录列表（status 含 completed/failed 语义）
      if (p.status === 'completed' || p.status === 'failed') {
        scheduleRefresh()
      }
    }),
  )

  unlistens.push(
    await listen<AsrRecordUpdatedPayload>('asr-record-updated', (e) => {
      const p = e.payload
      recordUpdatedCbs.forEach((cb) => {
        try {
          cb(p)
        } catch {
          /* ignore */
        }
      })
      // 默认节流刷新历史列表
      scheduleRefresh()
    }),
  )

  return () => {
    unlistens.forEach((fn) => fn?.())
    unlistens = []
    installed = false
    if (refreshTimer !== null) {
      window.clearTimeout(refreshTimer)
      refreshTimer = null
    }
  }
}

export interface UseAsrReturn {
  records: DeepReadonly<Ref<AsrRecord[]>>
  sessions: DeepReadonly<Ref<AsrSessionInfo[]>>
  config: DeepReadonly<Ref<AsrConfig | null>>
  loading: DeepReadonly<Ref<boolean>>
  listRecords: typeof listRecords
  getRecord: typeof getRecord
  deleteRecord: typeof deleteRecord
  searchRecords: typeof searchRecords
  searchSummaries: typeof searchSummaries
  startStreaming: typeof startStreaming
  pushAudio: typeof pushAudio
  finishStreaming: typeof finishStreaming
  cancelStreaming: typeof cancelStreaming
  transcribeFile: typeof transcribeFile
  generateSummary: typeof generateSummary
  updateRecord: typeof updateRecord
  loadConfig: typeof loadConfig
  saveConfig: typeof saveConfig
  listSessions: typeof listSessions
  install: typeof install
  onStreamChunk: typeof onStreamChunk
  onSessionStatus: typeof onSessionStatus
  onUploadProgress: typeof onUploadProgress
  onRecordUpdated: typeof onRecordUpdated
}

export function useAsr(): UseAsrReturn {
  return {
    records: readonly(records),
    sessions: readonly(sessions),
    config: readonly(config),
    loading: readonly(loading),
    listRecords,
    getRecord,
    deleteRecord,
    searchRecords,
    searchSummaries,
    startStreaming,
    pushAudio,
    finishStreaming,
    cancelStreaming,
    transcribeFile,
    generateSummary,
    updateRecord,
    loadConfig,
    saveConfig,
    listSessions,
    install,
    onStreamChunk,
    onSessionStatus,
    onUploadProgress,
    onRecordUpdated,
  }
}
