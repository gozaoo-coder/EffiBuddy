/**
 * useAskUser —— "AI 询问用户"对话框状态
 *
 * 监听后端 ask-user 事件(BusEvent::AskUser),弹出对话框收集用户选择。
 * - 单选(multi_select=false): 点击选项立即推进到下一题
 * - 多选(multi_select=true): 点击切换选中,需点击"提交"按钮推进
 *
 * 全部问题答完后,答案被格式化为用户消息,通过 send_message_stream 发送,
 * 在聊天列表中显示为一条用户气泡(展示简要摘要,如 "已选择: Q1 → 选项A; Q2 → 选项B、C")。
 *
 * 多会话安全:仅处理当前活跃会话的 ask-user 事件,其他会话的事件忽略(不排队)。
 * 每个 ChatWindow(会话页签)创建独立的 useAskUser 实例,互不干扰。
 */
import { ref, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { AskUserPayload, AskUserQuestion } from '../../types'
import type { useChatCore } from './useChatCore'
import type { useChatStreaming } from './useChatStreaming'

export function useAskUser(
  core: ReturnType<typeof useChatCore>,
  streaming: ReturnType<typeof useChatStreaming>,
) {
  // ---------- 响应式状态 ----------
  const visible = ref(false)
  const currentQuestions = ref<AskUserQuestion[]>([])
  const currentQuestionIndex = ref(0)
  const selectedOptions = ref<Set<number>>(new Set())
  const submitting = ref(false)

  /**
   * 已收集的各题答案(按问题顺序,Set 内为选项索引)。
   * 仅在 dialog 活跃期间有意义;advance() 写入,dismiss/resetAll 清空。
   * 每次写入都创建新数组/新 Set,保证响应式触发。
   */
  const collectedAnswers = ref<Set<number>[]>([])

  let unlisten: UnlistenFn | null = null

  // ---------- 事件订阅 ----------
  onMounted(async () => {
    unlisten = await listen<AskUserPayload>('ask-user', (e) => {
      const p = e.payload
      // 多会话过滤:仅处理当前活跃会话(每个 ChatWindow 只处理自己的会话)
      if (core.activeId.value !== p.conversation_id) return
      // 兜底:questions 至少 1 个
      if (!p.questions || p.questions.length === 0) return
      currentQuestions.value = p.questions
      currentQuestionIndex.value = 0
      selectedOptions.value = new Set()
      collectedAnswers.value = []
      submitting.value = false
      visible.value = true
    })
  })

  onUnmounted(() => {
    if (unlisten) {
      unlisten()
      unlisten = null
    }
  })

  // ---------- 选项交互 ----------

  /**
   * 选中选项。
   * 单选:设为 {index} 并立即推进到下一题(或发送)。
   * 多选:toggle 该索引,不自动推进(需点击"提交")。
   */
  function selectOption(index: number) {
    const q = currentQuestions.value[currentQuestionIndex.value]
    if (!q) return
    if (q.multi_select) {
      // 多选:toggle(创建新 Set 保证响应式)
      const next = new Set(selectedOptions.value)
      if (next.has(index)) next.delete(index)
      else next.add(index)
      selectedOptions.value = next
    } else {
      // 单选:设中即推进
      selectedOptions.value = new Set([index])
      advance()
    }
  }

  /**
   * 多选提交:验证至少选一项后推进到下一题(或发送)。
   */
  function submitCurrent() {
    if (selectedOptions.value.size === 0) {
      core.toast({ content: '请至少选择一项', type: 'info' })
      return
    }
    advance()
  }

  /**
   * 推进到下一题;若已是最后一题,触发 finalizeAndSend。
   * 内部方法,由 selectOption(单选)/ submitCurrent(多选)调用。
   */
  function advance() {
    const total = currentQuestions.value.length
    // 保存当前题的答案(创建新 Set 避免引用共享)
    const arr = collectedAnswers.value.slice()
    arr[currentQuestionIndex.value] = new Set(selectedOptions.value)
    collectedAnswers.value = arr

    if (currentQuestionIndex.value < total - 1) {
      currentQuestionIndex.value += 1
      selectedOptions.value = new Set()
    } else {
      // 最后一题,触发发送
      void finalizeAndSend()
    }
  }

  /**
   * 关闭对话框,不发送任何消息(用户取消)。
   */
  function dismiss() {
    visible.value = false
    currentQuestions.value = []
    currentQuestionIndex.value = 0
    selectedOptions.value = new Set()
    collectedAnswers.value = []
    submitting.value = false
  }

  // ---------- 答案格式化 ----------

  /**
   * 构造发送给后端的最终内容。
   * 格式:
   * 【已通过对话框回答】
   * 问题1: <q1 text>
   * 选择: <label1>
   *
   * 问题2: <q2 text>
   * 选择: <label1>、<label2>
   */
  function buildFinalContent(): string {
    const lines: string[] = ['【已通过对话框回答】']
    const total = currentQuestions.value.length
    for (let i = 0; i < total; i++) {
      const q = currentQuestions.value[i]
      const ans = collectedAnswers.value[i] ?? new Set<number>()
      const labels: string[] = []
      for (const idx of ans) {
        const opt = q.options[idx]
        if (opt) labels.push(opt.label)
      }
      const headerLabel = total > 1 ? `问题${i + 1}` : '问题'
      lines.push(`${headerLabel}: ${q.question}`)
      lines.push(`选择: ${labels.join('、')}`)
      if (i < total - 1) lines.push('')
    }
    return lines.join('\n')
  }

  /**
   * 构造用户气泡展示用的简要内容。
   * 例:"已选择: Q1 → 选项A; Q2 → 选项B、选项C"(单问题时省略 Q 前缀)。
   */
  function buildDisplayContent(): string {
    const parts: string[] = []
    const total = currentQuestions.value.length
    for (let i = 0; i < total; i++) {
      const q = currentQuestions.value[i]
      const ans = collectedAnswers.value[i] ?? new Set<number>()
      const labels: string[] = []
      for (const idx of ans) {
        const opt = q.options[idx]
        if (opt) labels.push(opt.label)
      }
      const prefix = total > 1 ? `Q${i + 1} ` : ''
      parts.push(`${prefix}→ ${labels.join('、')}`.trim())
    }
    return `已选择: ${parts.join('; ')}`
  }

  // ---------- 发送 ----------

  /**
   * 全部问题答完后,格式化答案并通过 send_message_stream 发送。
   * 复用 ChatComposer.send() 的编排:建会话 → 添加用户气泡 → invoke。
   * 注意:sending 由 agent-done / agent-stream-error 事件复位,不在此处重置。
   */
  async function finalizeAndSend() {
    // 防重入:AI 正在响应中
    if (core.sending.value) {
      core.toast({ content: 'AI 正在响应中，请稍候', type: 'info' })
      dismiss()
      return
    }

    // 确保有会话
    let id = core.activeId.value
    if (!id) {
      id = await core.ensureConversation()
      if (!id) {
        dismiss()
        return
      }
    }

    // 先构造内容(使用 collectedAnswers,在关闭对话框前完成)
    const finalContent = buildFinalContent()
    const displayContent = buildDisplayContent()

    core.sending.value = true
    submitting.value = true
    // 关闭对话框(visible=false 是外部赋值,不会触发 Dialog 的 update:visible,
    // 因此 dismiss 不会被调用,collectedAnswers 得以保留供上方 build* 使用)
    visible.value = false

    // 添加本地用户气泡(展示简要摘要,不含【已通过对话框回答】前缀)
    await streaming.addMessage({
      id: core.newId(),
      role: 'user',
      content: displayContent,
      timestamp: Date.now(),
    })

    try {
      await invoke('send_message_stream', {
        conversationId: id,
        content: finalContent,
      })
    } catch (e) {
      core.sending.value = false
      await streaming.addMessage({
        id: core.newId(),
        role: 'system',
        content: `请求失败：${e}`,
        timestamp: Date.now(),
      })
      core.toast({ content: `请求失败：${e}`, type: 'error' })
    } finally {
      // 重置对话框状态(sending 由 agent-done/agent-stream-error 控制,不在此处重置)
      currentQuestions.value = []
      currentQuestionIndex.value = 0
      selectedOptions.value = new Set()
      collectedAnswers.value = []
      submitting.value = false
      // 通知 App 刷新 SideNav 列表(消息数/时间更新)
      core.emit('conversation-changed')
    }
  }

  // ---------- 会话切换清空 ----------
  /** 会话切换/清空时:清空全部 ask-user 状态 */
  function resetAll() {
    visible.value = false
    currentQuestions.value = []
    currentQuestionIndex.value = 0
    selectedOptions.value = new Set()
    collectedAnswers.value = []
    submitting.value = false
  }

  return {
    visible,
    currentQuestions,
    currentQuestionIndex,
    selectedOptions,
    submitting,
    selectOption,
    submitCurrent,
    dismiss,
    resetAll,
  }
}
