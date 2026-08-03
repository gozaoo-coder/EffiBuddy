/**
 * 聊天事件编排:后端事件订阅 + 会话切换 watch + 生命周期
 *
 * 只做「订阅 → 分发」的编排工作,具体处理逻辑在各领域 store 中。
 * onMounted 注册一次全部 listen,onUnmounted 统一注销。
 */
import { watch, onMounted, onUnmounted } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  StreamTokenPayload,
  StreamErrorPayload,
  AgentStoppedPayload,
  AgentReasoningPayload,
  AgentToolCallPayload,
  AgentToolResultPayload,
  AgentAttachmentPayload,
  AgentBillingPayload,
  AgentUsagePayload,
  SubAgentEventPayload,
  CompressStatusPayload,
  CompressTokenPayload,
  CompressDonePayload,
  CompressErrorPayload,
} from '../../types'
import type { useChatCore } from './useChatCore'
import type { useChatStreaming } from './useChatStreaming'
import type { useChatCompression } from './useChatCompression'
import type { useTaskMode } from './useTaskMode'

export function useChatEvents(
  core: ReturnType<typeof useChatCore>,
  streaming: ReturnType<typeof useChatStreaming>,
  compression: ReturnType<typeof useChatCompression>,
  taskMode: ReturnType<typeof useTaskMode>,
) {
  let unlistens: UnlistenFn[] = []

  onMounted(async () => {
    // 初始加载当前会话(如果有)
    if (core.activeId.value) {
      await core.loadConversation()
    }

    // 获取当前激活模型信息,backend 切换时重新加载
    await core.loadActiveModelInfo()
    watch(
      () => core.props.backend,
      () => core.loadActiveModelInfo(),
      { immediate: false },
    )

    // 任务清单更新事件:右栏 / todo_write 工具每次增删改都会 emit
    // 多会话过滤:仅处理当前活跃会话(长程任务气泡与 todoTree 状态联动)
    unlistens.push(
      await listen<{ conversation_id: string }>('todo-tree-updated', (e) => {
        const p = e.payload
        if (core.activeId.value && p.conversation_id !== core.activeId.value) return
        void taskMode.loadTodoTree()
      }),
    )

    unlistens.push(
      await listen<StreamTokenPayload>('agent-token', async (e) => {
        const p = e.payload
        if (p.done) return
        if (core.activeId.value !== p.conversation_id) return
        await streaming.appendStreamToken(p.content)
      }),
    )

    unlistens.push(
      await listen<AgentReasoningPayload>('agent-reasoning', async (e) => {
        const p = e.payload
        if (core.activeId.value !== p.conversation_id) return
        await streaming.onReasoning(p.content)
      }),
    )

    unlistens.push(
      await listen<AgentToolCallPayload>('agent-tool-call', async (e) => {
        const p = e.payload
        if (core.activeId.value !== p.conversation_id) return
        // 长程任务模式:agent 调用 todo_write 建立任务树 → 聚合为任务气泡
        if (p.tool_name === 'todo_write') taskMode.markTaskTurn(streaming.streamingBubbleId.value)
        await streaming.onToolCall(p)
      }),
    )

    unlistens.push(
      await listen<AgentToolResultPayload>('agent-tool-result', async (e) => {
        const p = e.payload
        if (core.activeId.value !== p.conversation_id) return
        await streaming.onToolResult(p)
      }),
    )

    unlistens.push(
      await listen<AgentAttachmentPayload>('agent-attachment', async (e) => {
        const p = e.payload
        if (core.activeId.value !== p.conversation_id) return
        await streaming.onAttachment(p)
      }),
    )

    unlistens.push(
      await listen<SubAgentEventPayload>('sub-agent-event', async (e) => {
        const p = e.payload
        if (core.activeId.value !== p.conversation_id) return
        await streaming.onSubAgentEvent(p)
      }),
    )

    unlistens.push(
      await listen<AgentBillingPayload>('agent-billing', async (e) => {
        const p = e.payload
        if (core.activeId.value !== p.conversation_id) return
        await streaming.onBilling(p)
      }),
    )

    unlistens.push(
      await listen<AgentUsagePayload>('agent-usage', async (e) => {
        const p = e.payload
        if (core.activeId.value !== p.conversation_id) return
        // 用 API 真实 prompt_tokens 更新"上下文使用"仪表盘
        // （天然包含思维链、系统提示、记忆等整段 prompt，而非字符估算）
        core.setContextUsedTokens(p.input_tokens)
      }),
    )

    unlistens.push(
        await listen<StreamTokenPayload>('agent-done', async (e) => {
          const p = e.payload
          if (core.activeId.value !== p.conversation_id) return
          await streaming.finalizeStream(p.content)
          core.sending.value = false
          // 流结束:所有排队消息已被消费(续接循环),清零排队计数
          core.queuedCount.value = 0
        }),
          await listen<StreamErrorPayload>('agent-stream-error', async (e) => {
            const p = e.payload
            if (core.activeId.value !== p.conversation_id) return
            await streaming.addMessage({
              id: core.newId(),
              role: 'system',
              content: `流式错误：${p.error}`,
              timestamp: Date.now(),
            })
            core.toast({ content: `流式错误：${p.error}`, type: 'error' })
            streaming.streamingBubbleId.value = null
            core.sending.value = false
            // 流异常结束:排队消息可能未被消费,清零计数(消息仍在会话历史中,下次发送会并入)
            core.queuedCount.value = 0
            }),
        )

        // 用户点击「停止」:后端已持久化部分内容并 emit agent-stopped。
        // 这里收尾流式气泡(清 streamingBubbleId + 刷新 SideNav),并复位 sending。
        unlistens.push(
          await listen<AgentStoppedPayload>('agent-stopped', async (e) => {
            const p = e.payload
            if (core.activeId.value !== p.conversation_id) return
            await streaming.finalizeStream(p.content)
            core.sending.value = false
            // 后端停止时已清空排队队列(消息仍留在会话历史,下次发送会并入),同步清零 UI 计数
            core.queuedCount.value = 0
          }),
        )

      unlistens.push(
        await listen<CompressStatusPayload>('agent-compress-status', (e) => {
          const p = e.payload
          if (core.activeId.value !== p.conversation_id) return
          compression.compressStage.value = p.stage
          compression.compressStageMsg.value = p.message
        }),
        await listen<CompressTokenPayload>('agent-compress-token', (e) => {
          const p = e.payload
          if (core.activeId.value !== p.conversation_id) return
          compression.compressRawText.value += p.token
          // 实时解析已闭合的 <act> 块,让用户在 streaming 阶段就能看到决策
          compression.streamParsedActions.value = compression.parseStreamActs(
            compression.compressRawText.value,
          )
        }),
        await listen<CompressDonePayload>('agent-compress-done', (e) => {
          const p = e.payload
          if (core.activeId.value !== p.conversation_id) return
          compression.compressActions.value = p.actions
          compression.compressRawText.value = p.raw_text
          compression.compressElapsedMs.value = p.elapsed_ms
          compression.compressStage.value = 'done'
          // 记录本轮压缩等级（自动/手动统一走此事件；缺省按首次压缩处理）
          compression.compressLevel.value = p.level ?? 1
          // 写入真实 token 指标(done payload 携带,后端取自 API responses 的 usage)
          if (p.base_tokens != null) compression.compressBaseTokens.value = p.base_tokens
          if (p.current_tokens != null) compression.compressCurrentTokens.value = p.current_tokens
          // 清空流式解析(displayActions 会自动切换到 compressActions)
          compression.streamParsedActions.value = []
          // 同步刷新已存在状态(用于"上次压缩"展示)
          void compression.loadExistingCompression(p.conversation_id)
          // 完成 toast:让用户在关闭浮窗后也有反馈
          const stats = { keep: 0, hide: 0, replace: 0 }
          for (const a of p.actions) {
            if (a.method === 'keep') stats.keep++
            else if (a.method === 'hide') stats.hide++
            else if (a.method === 'replace') stats.replace++
          }
          const saved = compression.compressSavedInfo.value
          const savedText =
            saved && saved.savedTokens > 0
              ? ` · 已压缩 ${saved.savedTokens} tokens（${saved.percent}%）`
              : ''
          const levelText = p.level && p.level > 0 ? ` · 第 ${p.level} 级压缩` : ''
          core.toast({
            content: `压缩完成：保持 ${stats.keep} / 隐藏 ${stats.hide} / 替换 ${stats.replace}${levelText}${savedText}`,
            type: 'success',
          })
        }),
        await listen<CompressErrorPayload>('agent-compress-error', (e) => {
          const p = e.payload
          if (core.activeId.value !== p.conversation_id) return
          compression.compressError.value = p.error
          if (p.partial) compression.compressRawText.value = p.partial
          compression.compressStage.value = 'error'
        }),
      )
  })

  // conversationId 变化时加载对应会话消息 + 已有压缩状态
watch(
  () => core.props.conversationId,
  (id) => {
    core.setActiveId(id ?? null)
    void core.loadConversation()
    // 切换会话：清空上一会话的压缩运行态（阶段/流式文本/决策），避免残留影响新会话展示
    compression.resetCompressState()
    if (id) void compression.loadExistingCompression(id)
    else compression.compressExistingState.value = null
  },
)

  onUnmounted(() => {
    unlistens.forEach((fn) => fn?.())
    unlistens = []
  })
}
