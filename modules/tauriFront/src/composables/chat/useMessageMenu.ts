/**
 * 消息引用块 + 消息长按/右键菜单
 *
 * 引用块:composer 顶部展示的被引用消息 chip 列表,支持多条;
 * 点击 chip 主体 → 滚动到原消息并高亮闪烁;点击 x → 移除该引用。
 * 发送时把全部 chips 拼接为 [引用消息] 上下文块。
 *
 * 菜单:触摸长按 500ms 触发,鼠标用 contextmenu(参考 SideNav.vue 实现)。
 */
import { ref, computed } from 'vue'
import { animate } from 'animejs'
import type { Message, QuoteChip } from '../../types'
import type { MenuItemOption } from '../../components/basic'
import type { useChatCore } from './useChatCore'
import type { useChatStreaming } from './useChatStreaming'

export function useMessageMenu(
  core: ReturnType<typeof useChatCore>,
  streaming: ReturnType<typeof useChatStreaming>,
) {
  // ---------- 引用块 ----------
  const quoteChips = ref<QuoteChip[]>([])

  // 把消息摘要截断为前 40 字符 + …
  function makeSnippet(text: string): string {
    const t = (text ?? '').trim().replace(/\s+/g, ' ')
    if (t.length <= 40) return t
    return t.slice(0, 40) + '…'
  }

  // 添加引用:去重(同 messageId 不重复添加)
  function addQuote(m: Message) {
    if (quoteChips.value.some((q) => q.messageId === m.id)) {
      core.toast({ content: '已引用该消息', type: 'info' })
      return
    }
    quoteChips.value.push({
      messageId: m.id,
      snippet: makeSnippet(m.content),
      content: m.content,
      role: m.role,
    })
  }

  function removeQuote(messageId: string) {
    quoteChips.value = quoteChips.value.filter((q) => q.messageId !== messageId)
  }

  function clearQuotes() {
    quoteChips.value = []
  }

  // 滚动到原消息并用 animejs 做黄色边框闪烁(duration 1200ms)
  function scrollToMessage(id: string) {
    const el = document.getElementById('msg-' + id)
    if (!el) return
    el.scrollIntoView({ behavior: 'smooth', block: 'center' })
    animate(el, {
      boxShadow: [
        '0 0 0 0px rgba(255,213,79,0)',
        '0 0 0 3px rgba(255,213,79,0.9)',
        '0 0 0 0px rgba(255,213,79,0)',
      ],
      duration: 1200,
      ease: 'outQuad',
    })
  }

  /** 把引用块拼接到用户输入前面(发送时调用),格式:
   *    [引用消息]
   *    用户(id:xxx): 引用内容
   *    助手(id:yyy): 引用内容
   *
   *    用户实际输入
   */
  function buildQuoteContext(content: string): string {
    const chips = quoteChips.value
    if (chips.length === 0) return content
    const quoteBlock = chips
      .map((q) => {
        const roleLabel = q.role === 'user' ? '用户' : q.role === 'assistant' ? '助手' : '系统'
        return `${roleLabel}(id:${q.messageId}): ${q.content}`
      })
      .join('\n')
    return `[引用消息]\n${quoteBlock}\n\n${content}`
  }

  // ---------- 消息长按 / 右键菜单 ----------
  const msgMenuVisible = ref(false)
  const msgMenuPosition = ref<{ x: number; y: number } | null>(null)
  const msgMenuTarget = ref<Message | null>(null)
  let msgLongPressTimer: number | null = null

  function onMsgPointerDown(e: PointerEvent, m: Message) {
    // 鼠标用 contextmenu,触摸用长按
    if (e.pointerType === 'mouse') return
    msgLongPressTimer = window.setTimeout(() => {
      openMsgMenu(m, e.clientX, e.clientY)
    }, 500)
  }

  function onMsgPointerUp() {
    if (msgLongPressTimer) {
      clearTimeout(msgLongPressTimer)
      msgLongPressTimer = null
    }
  }

  function onMsgContextMenu(e: MouseEvent, m: Message) {
    e.preventDefault()
    openMsgMenu(m, e.clientX, e.clientY)
  }

  function openMsgMenu(m: Message, x: number, y: number) {
    msgMenuPosition.value = { x, y }
    msgMenuTarget.value = m
    msgMenuVisible.value = true
  }

  // Menu 项:引用、复制、删除(删除 danger + divided 分隔)
  const msgMenuItems = computed<MenuItemOption[]>(() => {
    const m = msgMenuTarget.value
    if (!m) return []
    return [
      { key: 'quote', label: '引用', icon: 'quote' },
      { key: 'copy', label: '复制', icon: 'file' },
      { key: 'delete', label: '删除', icon: 'delete', danger: true, divided: true },
    ]
  })

  function onMsgMenuSelect(item: MenuItemOption) {
    const m = msgMenuTarget.value
    msgMenuTarget.value = null
    if (!m) return
    switch (item.key) {
      case 'quote':
        addQuote(m)
        break
      case 'copy':
        void copyMessage(m)
        break
      case 'delete':
        removeMessageFromView(m.id)
        break
    }
  }

  // 复制消息内容到剪贴板
  async function copyMessage(m: Message) {
    try {
      await navigator.clipboard.writeText(m.content)
      core.toast({ content: '已复制', type: 'success' })
    } catch (e) {
      core.toast({ content: `复制失败：${e}`, type: 'error' })
    }
  }

  // 从视图移除消息(仅前端 UI,不调用后端)
  function removeMessageFromView(id: string) {
    const idx = core.messages.value.findIndex((m) => m.id === id)
    if (idx < 0) return
    core.messages.value.splice(idx, 1)
    // 同步清理 bubbleMeta / quoteChips
    delete streaming.bubbleMeta[id]
    quoteChips.value = quoteChips.value.filter((q) => q.messageId !== id)
    core.toast({ content: '已从视图移除', type: 'info' })
  }

  /** 会话切换/清空:清空引用与菜单状态 */
  function resetAll() {
    quoteChips.value = []
    msgMenuVisible.value = false
    msgMenuTarget.value = null
    msgMenuPosition.value = null
    onMsgPointerUp()
  }

  return {
    quoteChips,
    addQuote,
    removeQuote,
    clearQuotes,
    scrollToMessage,
    buildQuoteContext,
    msgMenuVisible,
    msgMenuPosition,
    msgMenuTarget,
    msgMenuItems,
    onMsgPointerDown,
    onMsgPointerUp,
    onMsgContextMenu,
    onMsgMenuSelect,
    copyMessage,
    removeMessageFromView,
    resetAll,
  }
}
