/**
 * 消息压缩(compress_messages_stream 流式命令)
 *
 * 设计:把压缩命令的流式事件实时渲染到浮窗:
 *  - 顶部阶段进度条(loading_conv → building_prompt → streaming → parsing → persisting → done)
 *  - 中部流式输出(markdown 实时渲染,含 <act> 块原始文本)
 *  - 底部决策列表(done 阶段展示解析后的 CompressionAction[],含 method/reason/涉及消息数)
 *  - 错误态:红色提示 + 已接收部分文本
 */
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type {
  CompressionAction,
  CompressionStage,
  CompressionState,
} from '../../types'
import type { useChatCore } from './useChatCore'

export function useChatCompression(core: ReturnType<typeof useChatCore>) {
  const compressionSheetOpen = ref(false)
  // 当前阶段
  const compressStage = ref<CompressionStage | 'idle'>('idle')
  // 阶段说明文本(来自 status 事件 message 字段)
  const compressStageMsg = ref('')
  // 流式累计的原始响应文本(含 <act> 块)
  const compressRawText = ref('')
  // 解析后的压缩决策列表(done 阶段填充)
  const compressActions = ref<CompressionAction[]>([])
  // 错误信息(error 阶段填充)
  const compressError = ref('')
  // 处理耗时(done 阶段填充,毫秒)
  const compressElapsedMs = ref(0)
  // 已存在的压缩状态(打开浮窗时从后端加载,用于展示历史压缩结果)
  const compressExistingState = ref<CompressionState | null>(null)
  const compressing = ref(false)
  // 流式实时解析的 actions(streaming 阶段使用,done 后用 compressActions 覆盖)
  const streamParsedActions = ref<CompressionAction[]>([])
  // 决策卡片展开状态:Set<key>,key = `${stage}-${index}`(区分 done/streaming/existing)
  const expandedActions = ref<Set<string>>(new Set())

  // ---------- 派生状态 ----------
  // 阶段进度映射:返回 0-1 的进度比例
  const compressProgress = computed(() => {
    const order: CompressionStage[] = [
      'loading_conv',
      'building_prompt',
      'streaming',
      'parsing',
      'persisting',
      'done',
    ]
    if (compressStage.value === 'error') return 0
    if (compressStage.value === 'idle') return 0
    const idx = order.indexOf(compressStage.value)
    if (idx < 0) return 0
    // done 算 1,其他按阶段顺序递增(最后一项 done 之前是 5/6)
    return Math.min(1, (idx + 1) / order.length)
  })

  // 阶段中文标签
  const compressStageLabel = computed(() => {
    const map: Record<CompressionStage | 'idle', string> = {
      idle: '待机',
      loading_conv: '加载会话',
      building_prompt: '构造 Prompt',
      streaming: '压缩 Agent 流式输出',
      parsing: '解析决策',
      persisting: '持久化',
      done: '完成',
      error: '错误',
    }
    return map[compressStage.value] ?? compressStage.value
  })

  // 决策统计:keep/hide/replace 各多少条
  const compressActionStats = computed(() => {
    const stats = { keep: 0, hide: 0, replace: 0, totalIds: 0 }
    for (const a of compressActions.value) {
      if (a.method === 'keep') stats.keep++
      else if (a.method === 'hide') stats.hide++
      else if (a.method === 'replace') stats.replace++
      stats.totalIds += a.message_ids.length
    }
    return stats
  })

  // 底栏压缩徽章:当前会话已压缩的消息条数(无需打开浮窗即可感知)
  const compressBadgeInfo = computed(() => {
    const state = compressExistingState.value
    if (!state || state.actions.length === 0) return null
    // 涉及消息总数(去重,避免同一 id 被多次决策时重复计数)
    const ids = new Set<string>()
    for (const a of state.actions) {
      for (const id of a.message_ids) ids.add(id)
    }
    return { count: ids.size, actionCount: state.actions.length }
  })

  // Token 节省量估算:基于 actions + 当前会话 messages 计算字符节省量
  // 估算规则:4 字符 ≈ 1 token(OpenAI 通用经验值)
  // - hide:节省 = content.length
  // - replace:节省 = max(0, content.length - new_content.length)
  // - keep:节省 = 0
  const compressSavedInfo = computed(() => {
    const actions =
      compressStage.value === 'done'
        ? compressActions.value
        : (compressExistingState.value?.actions ?? [])
    if (actions.length === 0) return null

    // 构建 id → message 索引(O(n)),用 Map 加速查表
    const msgMap = new Map<string, (typeof core.messages.value)[number]>()
    for (const m of core.messages.value) msgMap.set(m.id, m)

    let savedChars = 0
    let totalChars = 0
    // 反向遍历,先记录每个 id 的最终决策,再正向计算
    const finalAction = new Map<string, (typeof actions)[number]>()
    for (let i = actions.length - 1; i >= 0; i--) {
      const a = actions[i]
      for (const id of a.message_ids) {
        if (!finalAction.has(id)) finalAction.set(id, a)
      }
    }
    for (const [id, a] of finalAction) {
      const msg = msgMap.get(id)
      if (!msg) continue
      const origLen = msg.content?.length ?? 0
      totalChars += origLen
      if (a.method === 'hide') savedChars += origLen
      else if (a.method === 'replace' && a.new_content != null) {
        savedChars += Math.max(0, origLen - a.new_content.length)
      }
    }

    if (savedChars === 0) return { savedChars: 0, savedTokens: 0, percent: 0, totalChars }
    const savedTokens = Math.round(savedChars / 4)
    const percent = totalChars > 0 ? Math.round((savedChars / totalChars) * 100) : 0
    return { savedChars, savedTokens, percent, totalChars }
  })

  // ---------- 前端轻量 XML 解析器 ----------
  // 从流式文本中提取已闭合的 <act>...</act> 块,与后端 parse_compression_response
  // 逻辑一致,容错策略一致:未闭合的 <act> 跳过(仍在流式增长中)、
  // 缺少必要标签的块跳过、method 未知跳过。返回已成功解析的 actions(按出现顺序)。
  function parseStreamActs(text: string): CompressionAction[] {
    const actions: CompressionAction[] = []
    let pos = 0
    while (pos < text.length) {
      const relOpen = text.indexOf('<act>', pos)
      if (relOpen < 0) break
      const contentStart = relOpen + '<act>'.length
      const relClose = text.indexOf('</act>', contentStart)
      if (relClose < 0) break // 未闭合,等更多 token
      const block = text.slice(contentStart, relClose)
      pos = relClose + '</act>'.length
      const action = parseSingleAct(block)
      if (action) actions.push(action)
    }
    return actions
  }

  function parseSingleAct(block: string): CompressionAction | null {
    const reason = extractTag(block, 'reason')
    const method = extractTag(block, 'method')
    const idRaw = extractTag(block, 'completionId')
    if (!reason || !method || !idRaw) return null

    const messageIds = idRaw
      .replace(/^\[/, '')
      .replace(/\]$/, '')
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
    if (messageIds.length === 0) return null

    const m = method.trim()
    if (m === '保持') return { method: 'keep', reason: reason.trim(), message_ids: messageIds }
    if (m === '隐藏') return { method: 'hide', reason: reason.trim(), message_ids: messageIds }
    if (m === '替换') {
      const newContent = extractTag(block, 'newContent')
      if (!newContent) return null
      return {
        method: 'replace',
        reason: reason.trim(),
        message_ids: messageIds,
        new_content: newContent.trim(),
      }
    }
    return null
  }

  function extractTag(block: string, tag: string): string | null {
    const open = `<${tag}>`
    const close = `</${tag}>`
    const start = block.indexOf(open)
    if (start < 0) return null
    const end = block.indexOf(close, start + open.length)
    if (end < 0) return null
    return block.slice(start + open.length, end)
  }

  function toggleActionExpand(key: string) {
    const s = new Set(expandedActions.value)
    if (s.has(key)) s.delete(key)
    else s.add(key)
    expandedActions.value = s
  }

  // 根据 message_ids 在当前 messages 中查找原消息内容
  function findMessagesByIds(ids: string[]): { id: string; role: string; content: string }[] {
    const map = new Map(core.messages.value.map((m) => [m.id, m]))
    return ids
      .map((id) => map.get(id))
      .filter((m): m is NonNullable<typeof m> => !!m)
      .map((m) => ({
        id: m.id,
        role: m.role === 'user' ? '用户' : m.role === 'assistant' ? '助手' : '系统',
        content: m.content ?? '',
      }))
  }

  // ---------- 动作 ----------
  function resetCompressState() {
    compressStage.value = 'idle'
    compressStageMsg.value = ''
    compressRawText.value = ''
    compressActions.value = []
    compressError.value = ''
    compressElapsedMs.value = 0
    streamParsedActions.value = []
    expandedActions.value = new Set()
  }

  // 触发消息压缩:打开浮窗 + 调用流式命令(事件监听在 useChatEvents 注册一次)
  async function triggerCompress() {
    const id = core.activeId.value
    if (!id) {
      core.toast({ content: '请先选择会话', type: 'warn' })
      return
    }
    if (compressing.value) return

    resetCompressState()
    compressionSheetOpen.value = true
    compressing.value = true

    try {
      // 命令本身在流式完成后才返回;进度通过 agent-compress-* 事件实时推送
      await invoke('compress_messages_stream', { conversationId: id })
      // 命令返回即代表 done 事件已 emit,actions 已填充
      if (compressStage.value === 'done') {
        await core.loadConversation()
      }
    } catch (e) {
      // 若 error 事件未触发(如命令立即失败),手动设置错误态
      if (compressStage.value !== 'error') {
        compressStage.value = 'error'
        compressError.value = String(e)
      }
    } finally {
      compressing.value = false
    }
  }

  // 关闭压缩浮窗:仅 UI 操作,不打断进行中的压缩任务
  function closeCompressionSheet() {
    compressionSheetOpen.value = false
  }

  // 清除当前会话的压缩状态(恢复全量历史注入)
  async function clearCompression() {
    const id = core.activeId.value
    if (!id) return
    try {
      await invoke('clear_compression_state', { conversationId: id })
      compressExistingState.value = null
      compressActions.value = []
      core.toast({ content: '已清除压缩状态', type: 'success' })
      await core.loadConversation()
    } catch (e) {
      core.toast({ content: `清除失败：${e}`, type: 'error' })
    }
  }

  // 加载已有压缩状态(用于在浮窗打开时展示历史结果)
  async function loadExistingCompression(convId: string) {
    if (!convId) {
      compressExistingState.value = null
      return
    }
    try {
      const s = await invoke<CompressionState | null>('get_compression_state', {
        conversationId: convId,
      })
      compressExistingState.value = s
    } catch {
      compressExistingState.value = null
    }
  }

  /** 会话切换/清空:清空压缩状态 */
  function resetAll() {
    compressExistingState.value = null
    compressActions.value = []
    compressionSheetOpen.value = false
    resetCompressState()
  }

  return {
    compressionSheetOpen,
    compressStage,
    compressStageMsg,
    compressRawText,
    compressActions,
    compressError,
    compressElapsedMs,
    compressExistingState,
    compressing,
    streamParsedActions,
    expandedActions,
    compressProgress,
    compressStageLabel,
    compressActionStats,
    compressBadgeInfo,
    compressSavedInfo,
    parseStreamActs,
    toggleActionExpand,
    findMessagesByIds,
    resetCompressState,
    triggerCompress,
    closeCompressionSheet,
    clearCompression,
    loadExistingCompression,
    resetAll,
  }
}
