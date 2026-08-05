/**
 * useChatSend —— 发送编排（发送 / 停止 / 一键预设提示词）
 *
 * 职责：把「发送一条消息」的完整编排从 ChatComposer 抽出为独立 composable：
 *   引用拼接 → 建会话 → 流式调用 → AI 生成中排队 → 错误兜底
 * 供聊天输入栏（ChatComposer）与空态引导卡片（ChatHome 一键示例）复用，
 * 避免在两个组件里重复实现发送逻辑。
 *
 * 依赖：core / streaming / menu / autoscroll，
 * 均在 ChatWindow 组装各领域 store 后一起 provide（见 store.ts）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { useAutoScroll } from './useAutoScroll'
import type { useChatCore } from './useChatCore'
import type { useChatStreaming } from './useChatStreaming'
import type { useMessageMenu } from './useMessageMenu'

export function useChatSend(
  core: ReturnType<typeof useChatCore>,
  streaming: ReturnType<typeof useChatStreaming>,
  menu: ReturnType<typeof useMessageMenu>,
  autoscroll: ReturnType<typeof useAutoScroll>,
) {
  const { toast } = core

  /** 发送当前输入框内容（流式）。输入清空后的 textarea 高度回弹由 ChatComposer 的 watch 处理。 */
  async function send() {
    const content = core.input.value.trim()
    if (!content) return

    // 拼接引用上下文到 content 前面（引用前缀仅发给后端，用户气泡展示纯 content）
    const finalContent = menu.buildQuoteContext(content)

    // 用户主动发送：强制跟随到底部
    autoscroll.stickToBottom.value = true

    // 没有当前会话时新建一个（新建对话页签：id 为 null 或 __new_chat__ 哨兵）
    const id = await core.ensureConversation()
    if (!id) return

    // 快照当前是否生成中：AI 生成期间发送 → 排队插入下一个 completion 前（不启动新流）
    const isInterrupt = core.sending.value

    // 清空输入 + 清空引用
    core.input.value = ''
    menu.clearQuotes()

    // 用户气泡展示纯 content（不含引用前缀）
    await streaming.addMessage({
      id: core.newId(),
      role: 'user',
      content,
      timestamp: Date.now(),
    })

    if (isInterrupt) {
      // AI 仍在生成：消息排队，将在下一个 completion 前插入模型输入，
      // 由 send_message_stream 的续接循环 / rig hook 在下一轮消费。
      core.queuedCount.value++
      try {
        await invoke('queue_user_message', {
          conversationId: id,
          content: finalContent,
        })
      } catch (e) {
        core.queuedCount.value--
        await streaming.addMessage({
          id: core.newId(),
          role: 'system',
          content: `排队失败：${e}`,
          timestamp: Date.now(),
        })
        toast({ content: `排队失败：${e}`, type: 'error' })
      }
      return
    }
    core.sending.value = true
    // 推理设置（thinking 开关 + reasoning_effort）：关闭时传 null（后端不注入参数）
    const reasoning = core.thinking.value
      ? { thinking: true, effort: core.reasoningEffort.value }
      : null
    try {
      await invoke('send_message_stream', {
        conversationId: id,
        content: finalContent,
        reasoning,
      })
    } catch (e) {
      core.sending.value = false
      await streaming.addMessage({
        id: core.newId(),
        role: 'system',
        content: `请求失败：${e}`,
        timestamp: Date.now(),
      })
      toast({ content: `请求失败：${e}`, type: 'error' })
    }
  }

  /** 停止生成（暂停对话）：后端 stop_agent 取消当前会话的流式驱动 task。 */
  async function stopGenerating() {
    const id = core.activeId.value
    if (!id) return
    try {
      await invoke('stop_agent', { conversationId: id })
    } catch (e) {
      toast({ content: `停止失败：${e}`, type: 'error' })
    }
  }

  /** 一键发送预设提示词（空态引导卡片用）：填入输入框并立即发送。 */
  async function sendPrompt(text: string) {
    core.input.value = text
    await send()
  }

  return { send, stopGenerating, sendPrompt }
}

export type UseChatSendReturn = ReturnType<typeof useChatSend>
