<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed, watch, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { animate } from 'animejs'
import MarkdownRender from 'markstream-vue'
import { useTheme } from '../composables/useTheme'
import { Button, IconButton, BindSheet, Chips, Icon, Menu, ContextRing, useToast, type MenuItemOption } from './basic'
import ReasoningBox from './ReasoningBox.vue'
import ToolCallGroup from './ToolCallGroup.vue'
import type {
  Message,
  Conversation,
  Attachment,
  StreamTokenPayload,
  StreamErrorPayload,
  AgentReasoningPayload,
  AgentToolCallPayload,
  AgentToolResultPayload,
  AgentAttachmentPayload,
  ToolCallRecord,
  PickedFile,
  QuoteChip,
} from '../types'

// 后端名称（来自 App.vue 顶部模型药丸）+ 当前会话 id（由 App 传入）
const props = defineProps<{
  backend?: string
  conversationId?: string | null
}>()

const emit = defineEmits<{
  (e: 'update:conversation-id', id: string | null): void
  (e: 'conversation-changed'): void
}>()

// 主题：用于把 is-dark 传给 MarkdownRender，确保代码块/深色样式正确
const { resolvedTheme } = useTheme()
const isDark = computed(() => resolvedTheme.value === 'dark')
const { toast } = useToast()

// ---------- 状态 ----------
// activeId：当前正在交互的会话 id（流式事件匹配用）。
// 与 props.conversationId 同步，但在新建会话时可立即赋值，不等 App 回传。
const activeId = ref<string | null>(props.conversationId ?? null)
const messages = ref<Message[]>([])
const input = ref('')
const sending = ref(false)
const scroller = ref<HTMLElement | null>(null)
const streamingBubbleId = ref<string | null>(null) // 当前正在流式填充的气泡 id
// 工具结果到达后置位：下一个文本/推理 token 应新建气泡，实现"每段答复独立气泡"
// 规则：文本 + 工具调用归同一气泡；工具结果后下一段文本/推理新建气泡
// 连续多个工具无中间文本时，工具调用仍追加到当前气泡（视觉连贯）
const needNewBubbleAfterTool = ref(false)

// 自动滚动控制：markstream-vue 的 smooth-streaming 内部异步渲染，
// nextTick 后 DOM 可能尚未增长，scrollHeight 是旧值导致 scrollBottom 失效。
// 改用 MutationObserver 监听 scroller 子树变化，配合 requestAnimationFrame
// 节流地跟随底部。stickToBottom 跟踪用户滚动位置，上滑阅读时暂停跟随。
const stickToBottom = ref(true)
let mutationObserver: MutationObserver | null = null
let scrollRafId: number | null = null

// 每个助手气泡的元数据：reasoning / tool calls（流式期间累积，不持久化）
interface BubbleMeta {
  reasoning: string
  isThinking: boolean
  toolCalls: ToolCallRecord[]
}
const bubbleMeta = reactive<Record<string, BubbleMeta>>({})

// 附件图片 base64 data URL 缓存：attachment.id -> data URL
// read_attachment 命令把图片文件编码成 data URL 返回，避免 Tauri 2 资源协议配置。
// 历史消息和实时生成共用此缓存。
const attachmentUrls = reactive<Record<string, string>>({})

// 获取某条消息的元数据（若不存在返回 null）
function getMeta(id: string): BubbleMeta | null {
  return bubbleMeta[id] ?? null
}

// 确保某 bubble 的 meta 存在
function ensureMeta(id: string): BubbleMeta {
  if (!bubbleMeta[id]) {
    bubbleMeta[id] = {
      reasoning: '',
      isThinking: false,
      toolCalls: [],
    }
  }
  return bubbleMeta[id]
}

// 底部工具/附件 Sheet
const toolSheetOpen = ref(false)

// 会话级工作区路径：None 表示未设置（回退到技能级或进程默认）
// 优先级：会话级 > 技能级（apply_skill 写入） > 进程默认 cwd
const workingDir = ref<string | null>(null)
const workingDirSheetOpen = ref(false)

// ---------- 引用块（任务 C）----------
// composer 顶部展示的被引用消息 chip 列表，支持多条
// 点击 chip 主体 → 滚动到原消息并高亮闪烁；点击 x → 移除该引用
const quoteChips = ref<QuoteChip[]>([])

// 把消息摘要截断为前 40 字符 + …
function makeSnippet(text: string): string {
  const t = (text ?? '').trim().replace(/\s+/g, ' ')
  if (t.length <= 40) return t
  return t.slice(0, 40) + '…'
}

// 添加引用：去重（同 messageId 不重复添加）
function addQuote(m: Message) {
  if (quoteChips.value.some((q) => q.messageId === m.id)) {
    toast({ content: '已引用该消息', type: 'info' })
    return
  }
  quoteChips.value.push({
    messageId: m.id,
    snippet: makeSnippet(m.content),
    content: m.content,
    role: m.role,
  })
}

// 移除指定引用
function removeQuote(messageId: string) {
  quoteChips.value = quoteChips.value.filter((q) => q.messageId !== messageId)
}

// 滚动到原消息并用 animejs 做黄色边框闪烁（duration 1200ms）
function scrollToMessage(id: string) {
  const el = document.getElementById('msg-' + id)
  if (!el) return
  el.scrollIntoView({ behavior: 'smooth', block: 'center' })
  // animejs 闪烁：透明 → 黄色 → 透明
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

// ---------- 消息长按 / 右键菜单（任务 C）----------
// 触摸长按 500ms 触发，鼠标用 contextmenu（参考 SideNav.vue 实现）
const msgMenuVisible = ref(false)
const msgMenuPosition = ref<{ x: number; y: number } | null>(null)
const msgMenuTarget = ref<Message | null>(null)
let msgLongPressTimer: number | null = null

function onMsgPointerDown(e: PointerEvent, m: Message) {
  // 鼠标用 contextmenu，触摸用长按
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

// Menu 项：引用、复制、删除（删除 danger + divided 分隔）
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
    toast({ content: '已复制', type: 'success' })
  } catch (e) {
    toast({ content: `复制失败：${e}`, type: 'error' })
  }
}

// 从视图移除消息（仅前端 UI，不调用后端）
function removeMessageFromView(id: string) {
  const idx = messages.value.findIndex((m) => m.id === id)
  if (idx < 0) return
  messages.value.splice(idx, 1)
  // 同步清理 bubbleMeta 与 quoteChips
  delete bubbleMeta[id]
  quoteChips.value = quoteChips.value.filter((q) => q.messageId !== id)
  toast({ content: '已从视图移除', type: 'info' })
}

// ---------- composer 升级（任务 D）----------
const composerFocused = ref(false)
const textareaRef = ref<HTMLTextAreaElement | null>(null)
const composerInnerRef = ref<HTMLElement | null>(null)

// 上下文使用统计：粗略 4 字符 = 1 token，max ~32K tokens = 128K 字符
const contextMaxChars = 128000
const contextUsedChars = computed(() =>
  messages.value.reduce((sum, m) => sum + (m.content?.length ?? 0), 0),
)
const contextUsedTokens = computed(() => Math.ceil(contextUsedChars.value / 4))
const contextMaxTokens = computed(() => Math.ceil(contextMaxChars / 4))

// 上下文管理 Sheet（含消息压缩按钮，任务 B 并行实现后端命令）
const contextSheetOpen = ref(false)
const compressing = ref(false)

// 触发消息压缩：调用后端 compress_messages 命令（任务 B 实现）
// 前端代码先写好，vue-tsc 不检查 invoke 命令是否存在
async function triggerCompress() {
  const id = activeId.value
  if (!id) {
    toast({ content: '请先选择会话', type: 'warn' })
    return
  }
  if (compressing.value) return
  compressing.value = true
  try {
    await invoke('compress_messages', { conversationId: id })
    toast({ content: '压缩完成', type: 'success' })
    await loadConversation()
    contextSheetOpen.value = false
  } catch (e) {
    toast({ content: `压缩失败：${e}`, type: 'error' })
  } finally {
    compressing.value = false
  }
}

// composer-inner 高度动画（关键：禁止 height: fit-content，用 animejs 动画）
function autoResize() {
  const ta = textareaRef.value
  if (!ta) return
  // 当前高度（animejs 动画起点）
  const currentHeight = ta.offsetHeight
  // 临时设为 auto 测量自然内容高度（同步操作，不触发重绘）
  ta.style.height = 'auto'
  const naturalHeight = ta.scrollHeight
  // 立即恢复当前高度，避免视觉跳变
  ta.style.height = currentHeight + 'px'
  // 目标高度：不超过 120px
  const targetHeight = Math.min(naturalHeight, 120)
  // 强制 reflow，确保 animejs 起点正确
  void ta.offsetHeight
  animate(ta, {
    height: [currentHeight + 'px', targetHeight + 'px'],
    duration: 200,
    ease: 'out(3)',
  })
}

let unlistens: UnlistenFn[] = []

// ---------- 工具函数 ----------
function newId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function scrollBottom() {
  const el = scroller.value
  if (el && stickToBottom.value) el.scrollTop = el.scrollHeight
}

// 节流跟随底部：MutationObserver 触发时合并到下一帧统一滚动，避免高频 token 抖动
function scheduleFollowBottom() {
  if (!stickToBottom.value || scrollRafId !== null) return
  scrollRafId = requestAnimationFrame(() => {
    scrollRafId = null
    const el = scroller.value
    if (el && stickToBottom.value) el.scrollTop = el.scrollHeight
  })
}

// 滚动事件：用户上滑超过阈值时停止跟随，滑回底部时恢复
function onScrollerScroll() {
  const el = scroller.value
  if (!el) return
  const distance = el.scrollHeight - el.scrollTop - el.clientHeight
  stickToBottom.value = distance < 80
}

// 在 scroller 挂载/卸载时绑定/解绑 observer 与滚动监听
function attachScroller(el: HTMLElement | null, oldEl?: HTMLElement | null) {
  if (oldEl) {
    oldEl.removeEventListener('scroll', onScrollerScroll)
  }
  if (mutationObserver) {
    mutationObserver.disconnect()
    mutationObserver = null
  }
  if (!el) return
  el.addEventListener('scroll', onScrollerScroll, { passive: true })
  mutationObserver = new MutationObserver(scheduleFollowBottom)
  mutationObserver.observe(el, {
    childList: true,
    subtree: true,
    characterData: true,
  })
}

// 是否显示空状态首页
const isEmptyHome = computed(() => messages.value.length === 0 && !sending.value)

// ---------- 会话加载 ----------
// conversationId 变化时加载对应会话消息
watch(
  () => props.conversationId,
  (id) => {
    activeId.value = id ?? null
    loadConversation()
  },
)

// scroller 是 v-else 渲染的元素，首次发消息时才挂载。
// 在此 watch 中绑定 MutationObserver，确保流式期间 DOM 增长能可靠触发滚动。
watch(scroller, (el, oldEl) => {
  attachScroller(el, oldEl ?? null)
})

async function loadConversation() {
  const id = activeId.value
  if (!id) {
    messages.value = []
    // 清空 meta 与附件缓存
    Object.keys(bubbleMeta).forEach((k) => delete bubbleMeta[k])
    Object.keys(attachmentUrls).forEach((k) => delete attachmentUrls[k])
    workingDir.value = null
    // 清空引用块，避免残留上一会话的引用
    quoteChips.value = []
    return
  }
  try {
    const conv = await invoke<Conversation | null>('get_conversation', { id })
    messages.value = conv?.messages ?? []
    // 历史会话不携带 reasoning/tools 元数据，清空
    Object.keys(bubbleMeta).forEach((k) => delete bubbleMeta[k])
    // 切换会话时清空旧附件缓存，避免上一会话的 data URL 残留占用内存
    Object.keys(attachmentUrls).forEach((k) => delete attachmentUrls[k])
    // 清空引用块，避免残留上一会话的引用
    quoteChips.value = []
    // 加载会话级工作区
    workingDir.value = conv?.working_dir ?? null
    // 历史消息可能携带 attachments（如历史 image_gen 结果），回填 base64
    await loadConversationAttachments()
    await nextTick()
    scrollBottom()
  } catch (e) {
    console.warn('get_conversation failed', e)
    messages.value = []
    Object.keys(bubbleMeta).forEach((k) => delete bubbleMeta[k])
    Object.keys(attachmentUrls).forEach((k) => delete attachmentUrls[k])
    workingDir.value = null
  }
}

// ---------- 会话级工作区管理 ----------
// 优先级：会话级 > 技能级（apply_skill 写入） > 进程默认 cwd
async function pickWorkingDir() {
  const id = activeId.value
  if (!id) {
    toast({ content: '请先选择或新建会话', type: 'warn' })
    return
  }
  try {
    const path = await invoke<string | null>('pick_directory')
    if (path) {
      await invoke('set_conversation_working_dir', {
        conversationId: id,
        workingDir: path,
      })
      workingDir.value = path
      toast({ content: `已设置工作区：${path}`, type: 'success' })
    }
  } catch (e) {
    toast({ content: `设置工作区失败：${e}`, type: 'error' })
  }
}

async function clearWorkingDir() {
  const id = activeId.value
  if (!id) return
  try {
    await invoke('set_conversation_working_dir', {
      conversationId: id,
      workingDir: null,
    })
    workingDir.value = null
    toast({ content: '已清除工作区', type: 'success' })
  } catch (e) {
    toast({ content: `清除工作区失败：${e}`, type: 'error' })
  }
}

// ---------- 消息渲染 ----------
async function addMessage(msg: Message) {
  messages.value.push(msg)
  await nextTick()
  animate('.msg-bubble:last-child', {
    opacity: [0, 1],
    translateY: [10, 0],
    duration: 400,
    easing: 'easeOutQuad',
  })
  scrollBottom()
}

async function appendStreamToken(token: string) {
  // 工具结果后下一段文本应新建气泡（实现"每段答复独立气泡"）
  if (needNewBubbleAfterTool.value) {
    streamingBubbleId.value = null
    needNewBubbleAfterTool.value = false
  }
  if (!streamingBubbleId.value) {
    streamingBubbleId.value = newId()
    await addMessage({
      id: streamingBubbleId.value,
      role: 'assistant',
      content: token,
      timestamp: Date.now(),
    })
  } else {
    const target = messages.value.find((m) => m.id === streamingBubbleId.value)
    if (target) {
      target.content += token
    }
    await nextTick()
    scrollBottom()
  }
  // 收到文本 token 表示推理阶段已结束
  if (streamingBubbleId.value) {
    const meta = bubbleMeta[streamingBubbleId.value]
    if (meta && meta.isThinking) meta.isThinking = false
  }
}

// ---------- 推理事件 ----------
async function onReasoning(content: string) {
  // 工具结果后新一轮推理也应新建气泡（新一轮思考 = 新一段答复）
  if (needNewBubbleAfterTool.value) {
    streamingBubbleId.value = null
    needNewBubbleAfterTool.value = false
  }
  if (!streamingBubbleId.value) {
    // 没有气泡时先创建一个空的 assistant 气泡
    streamingBubbleId.value = newId()
    await addMessage({
      id: streamingBubbleId.value,
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
    })
  }
  const meta = ensureMeta(streamingBubbleId.value)
  meta.isThinking = true
  meta.reasoning += content
  await nextTick()
  scrollBottom()
}

// ---------- 工具调用事件 ----------
async function onToolCall(call: AgentToolCallPayload) {
  if (!streamingBubbleId.value) {
    streamingBubbleId.value = newId()
    await addMessage({
      id: streamingBubbleId.value,
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
    })
  }
  const meta = ensureMeta(streamingBubbleId.value)
  // 收到 tool call 表示推理阶段结束
  meta.isThinking = false
  meta.toolCalls.push({
    call_id: call.call_id,
    tool_name: call.tool_name,
    arguments: call.arguments,
    result: null,
    is_error: false,
    pending: true,
  })
  await nextTick()
  scrollBottom()
}

// ---------- 工具结果事件 ----------
async function onToolResult(result: AgentToolResultPayload) {
  if (!streamingBubbleId.value) return
  const meta = bubbleMeta[streamingBubbleId.value]
  if (!meta) return
  const target = meta.toolCalls.find((c) => c.call_id === result.call_id)
  if (target) {
    target.result = result.output
    target.is_error = result.is_error
    target.pending = false
  }
  // 标记：下一个文本/推理 token 应新建气泡
  // 实现"文本+工具归同一气泡，工具结果后下段答复独立气泡"
  needNewBubbleAfterTool.value = true
  await nextTick()
  scrollBottom()
}

// ---------- 附件图片渲染 ----------
// 调用 read_attachment 命令把图片文件读成 base64 data URL，缓存到 attachmentUrls。
// 一次加载后多次复用（同附件 id 在流式与历史加载间不重复请求）。
async function loadAttachmentDataUrl(att: Attachment) {
  if (attachmentUrls[att.id]) return
  try {
    const dataUrl = await invoke<string>('read_attachment', { path: att.path })
    attachmentUrls[att.id] = dataUrl
  } catch (e) {
    console.warn('read_attachment failed', att.path, e)
  }
}

// 批量加载一组消息的所有附件（用于 loadConversation 历史回填）
async function loadConversationAttachments() {
  const tasks: Promise<void>[] = []
  for (const m of messages.value) {
    if (m.attachments && m.attachments.length > 0) {
      for (const att of m.attachments) {
        if (!attachmentUrls[att.id]) tasks.push(loadAttachmentDataUrl(att))
      }
    }
  }
  if (tasks.length > 0) await Promise.all(tasks)
}

// ---------- 图片附件事件 ----------
// image_gen 工具成功生成图片时，后端 emit "agent-attachment" 实时推送 Attachment。
// 前端立即把它挂到当前流式气泡上并加载 base64，用户可在文本生成完成前就看到图片。
async function onAttachment(payload: AgentAttachmentPayload) {
  if (!streamingBubbleId.value) return
  const target = messages.value.find((m) => m.id === streamingBubbleId.value)
  if (!target) return
  if (!target.attachments) target.attachments = []
  // 防止重复推送（同 id 二次到达）
  if (!target.attachments.some((a) => a.id === payload.attachment.id)) {
    target.attachments.push(payload.attachment)
  }
  await loadAttachmentDataUrl(payload.attachment)
  await nextTick()
  scrollBottom()
}

// ---------- 图片预览 ----------
// 点击消息内图片打开全屏预览（Teleport 到 body），再次点击遮罩或按 Esc 关闭
const previewState = reactive({
  visible: false,
  url: '',
  name: '',
})

function openImagePreview(url: string, name: string) {
  if (!url) return
  previewState.url = url
  previewState.name = name
  previewState.visible = true
}

function closeImagePreview() {
  previewState.visible = false
  previewState.url = ''
  previewState.name = ''
}

function onPreviewKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') closeImagePreview()
}

async function finalizeStream(full: string) {
  // 分气泡流式：工具结果后已新建多个气泡，full 是全部文本拼接，不能覆盖。
  // 仅在"没有任何气泡"的回退场景下用 full 创建一条消息（理论上不会发生，
  // 因为首个 token 就会创建气泡，但作为防御性兜底）。
  if (!streamingBubbleId.value) {
    if (full) {
      await addMessage({
        id: newId(),
        role: 'assistant',
        content: full,
        timestamp: Date.now(),
      })
    }
  }
  // 注：不再用 full 覆盖最后一个气泡 content，因为分气泡场景下
  // 各气泡已通过 appendStreamToken 正确累积自己的片段，full 是整体拼接会破坏分段
  streamingBubbleId.value = null
  needNewBubbleAfterTool.value = false
  // 流式结束后通知 App 刷新 SideNav 列表（消息数/时间更新）
  emit('conversation-changed')
}

// ---------- 发送（流式） ----------
async function send() {
  const content = input.value.trim()
  if (!content || sending.value) return

  // 拼接引用上下文到 content 前面（任务 C.3）
  // 格式：
  //   [引用消息]
  //   用户(id:xxx): 引用内容
  //   助手(id:yyy): 引用内容
  //
  //   用户实际输入
  let finalContent = content
  const chips = quoteChips.value
  if (chips.length > 0) {
    const quoteBlock = chips
      .map((q) => {
        const roleLabel = q.role === 'user' ? '用户' : q.role === 'assistant' ? '助手' : '系统'
        return `${roleLabel}(id:${q.messageId}): ${q.content}`
      })
      .join('\n')
    finalContent = `[引用消息]\n${quoteBlock}\n\n${content}`
  }

  // 用户主动发送：强制跟随到底部
  stickToBottom.value = true

  // 没有当前会话时新建一个
  let id = activeId.value
  if (!id) {
    try {
      id = await invoke<string>('create_conversation')
      activeId.value = id
      emit('update:conversation-id', id)
      emit('conversation-changed')
    } catch (e) {
      toast({ content: `新建会话失败：${e}`, type: 'error' })
      return
    }
  }

  sending.value = true
  input.value = ''
  // 清空引用块（发送后）
  quoteChips.value = []
  // 重置 textarea 高度
  await nextTick()
  if (textareaRef.value) {
    animate(textareaRef.value, {
      height: [textareaRef.value.offsetHeight + 'px', '40px'],
      duration: 200,
      ease: 'out(3)',
    })
  }

  // 用户气泡展示纯 content（不含引用前缀，引用前缀仅发给后端）
  await addMessage({
    id: newId(),
    role: 'user',
    content,
    timestamp: Date.now(),
  })

  try {
    await invoke('send_message_stream', {
      conversationId: id,
      content: finalContent,
    })
  } catch (e) {
    sending.value = false
    await addMessage({
      id: newId(),
      role: 'system',
      content: `请求失败：${e}`,
      timestamp: Date.now(),
    })
    toast({ content: `请求失败：${e}`, type: 'error' })
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault()
    send()
  }
}

// ---------- 快捷胶囊 ----------
const quickActions = [
  { label: 'PPT', icon: 'image' },
  { label: '集群', icon: 'globe' },
  { label: '网站', icon: 'globe' },
  { label: '深度研究', icon: 'search' },
]

function applyQuickAction(label: string) {
  input.value = `帮我做一个${label}相关的方案`
}

// ---------- 底部工具 Sheet ----------
const toolCategories = [
  { label: '拍照', icon: 'camera' },
  { label: '照片', icon: 'image' },
  { label: '本地文件', icon: 'folder' },
  { label: '微信文件', icon: 'wechat' },
]

const pluginItems = [
  { label: '插件', desc: '接入 App 和数据库，帮你自动操作', icon: 'plug' },
  { label: '技能', desc: '复用专业能力，稳定处理特定任务', icon: 'tool' },
]

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

// 工具卡片点击：拍照/照片/本地文件接真实 command
async function onToolClick(label: string) {
  toolSheetOpen.value = false
  try {
    let file: PickedFile | null = null
    if (label === '拍照') {
      file = await invoke<PickedFile>('capture_photo')
    } else if (label === '照片') {
      file = await invoke<PickedFile>('pick_image')
    } else if (label === '本地文件') {
      file = await invoke<PickedFile>('pick_file')
    } else {
      toast({ content: `${label} 功能即将上线`, type: 'info' })
      return
    }
    if (file) {
      toast({
        content: `已选择：${file.name}（${formatFileSize(file.size)}）`,
        type: 'success',
      })
    }
  } catch (e) {
    toast({ content: `${label}失败：${e}`, type: 'error' })
  }
}

// ---------- 事件订阅 ----------
onMounted(async () => {
  // 初始加载当前会话（如果有）
  if (activeId.value) {
    await loadConversation()
  }

  unlistens.push(
    await listen<StreamTokenPayload>('agent-token', async (e) => {
      const p = e.payload
      if (p.done) return
      if (activeId.value && p.conversation_id !== activeId.value) return
      await appendStreamToken(p.content)
    }),
  )

  unlistens.push(
    await listen<AgentReasoningPayload>('agent-reasoning', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onReasoning(p.content)
    }),
  )

  unlistens.push(
    await listen<AgentToolCallPayload>('agent-tool-call', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onToolCall(p)
    }),
  )

  unlistens.push(
    await listen<AgentToolResultPayload>('agent-tool-result', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onToolResult(p)
    }),
  )

  unlistens.push(
    await listen<AgentAttachmentPayload>('agent-attachment', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onAttachment(p)
    }),
  )

  unlistens.push(
    await listen<StreamTokenPayload>('agent-done', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await finalizeStream(p.content)
      sending.value = false
    }),
  )

  unlistens.push(
    await listen<StreamErrorPayload>('agent-stream-error', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await addMessage({
        id: newId(),
        role: 'system',
        content: `流式错误：${p.error}`,
        timestamp: Date.now(),
      })
      toast({ content: `流式错误：${p.error}`, type: 'error' })
      streamingBubbleId.value = null
      sending.value = false
    }),
  )

  // 图片预览 Esc 关闭
  window.addEventListener('keydown', onPreviewKeydown)
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
  if (mutationObserver) {
    mutationObserver.disconnect()
    mutationObserver = null
  }
  if (scrollRafId !== null) {
    cancelAnimationFrame(scrollRafId)
    scrollRafId = null
  }
  if (msgLongPressTimer) {
    clearTimeout(msgLongPressTimer)
    msgLongPressTimer = null
  }
  window.removeEventListener('keydown', onPreviewKeydown)
})
</script>

<template>
  <div class="chat-window">
    <!-- 聊天主区（侧栏已提升为 App 级 SideNav 抽屉） -->
    <section class="chat-main">
      <!-- 空状态首页：Kimi 风格中央品牌区 + 快捷胶囊 -->
      <div v-if="isEmptyHome" class="home-empty">
        <div class="home-brand">
          <div class="home-logo">
            <span class="home-logo-text">Effi</span>
            <span class="home-logo-icon"><Icon name="robot" :size="48" /></span>
            <span class="home-logo-text">Buddy</span>
          </div>
          <div class="home-subtitle">
            {{ props.backend && props.backend !== 'unknown' ? props.backend : 'Kimi K3: 2.8万亿参数' }}
          </div>
          <div class="home-subtitle secondary">为你进化</div>
        </div>

        <div class="home-actions">
          <Chips
            v-for="action in quickActions"
            :key="action.label"
            :label="action.label"
            size="md"
            @click="applyQuickAction(action.label)"
          >
            <template #icon><Icon :name="action.icon" :size="16" /></template>
          </Chips>
        </div>
      </div>

      <!-- 消息列表 -->
      <div v-else ref="scroller" class="msg-list">
        <div
          v-for="m in messages"
          :id="'msg-' + m.id"
          :key="m.id"
          class="msg-bubble"
          :class="[`role-${m.role}`, { streaming: m.id === streamingBubbleId }]"
          @pointerdown="onMsgPointerDown($event, m)"
          @pointerup="onMsgPointerUp"
          @pointerleave="onMsgPointerUp"
          @pointercancel="onMsgPointerUp"
          @contextmenu="onMsgContextMenu($event, m)"
        >
          <template v-if="m.role === 'assistant'">
            <!-- 推理折叠框：仅在存在 reasoning 时渲染 -->
            <ReasoningBox
              v-if="getMeta(m.id)?.reasoning"
              :content="getMeta(m.id)!.reasoning"
              :is-thinking="getMeta(m.id)!.isThinking"
            />
            <!-- 工具调用提示组：仅在存在 tool calls 时渲染 -->
            <ToolCallGroup
              v-if="getMeta(m.id)?.toolCalls.length"
              :calls="getMeta(m.id)!.toolCalls"
            />
            <!-- 正文：仅在内容非空时渲染（思考/工具阶段内容可能为空） -->
            <MarkdownRender
              v-if="m.content"
              mode="chat"
              :content="m.content"
              :final="m.id !== streamingBubbleId"
              :is-dark="isDark"
              :fade="false"
              smooth-streaming="auto"
              :code-block-props="{
                theme: { light: 'vitesse-light', dark: 'vitesse-dark' },
              }"
            />
            <!-- 附件图片区域：image_gen 工具生成的图片在此渲染 -->
            <div
              v-if="m.attachments && m.attachments.length > 0"
              class="msg-attachments"
            >
              <div
                v-for="att in m.attachments"
                :key="att.id"
                class="msg-attachment"
                :class="`att-${att.kind}`"
              >
                <img
                  v-if="attachmentUrls[att.id]"
                  :src="attachmentUrls[att.id]"
                  :alt="att.name"
                  class="msg-attachment-img"
                  loading="lazy"
                  @click="openImagePreview(attachmentUrls[att.id], att.name)"
                />
                <div v-else class="msg-attachment-loading">
                  <Icon name="image" :size="20" />
                  <span>加载中…</span>
                </div>
                <div class="msg-attachment-meta">{{ att.name }}</div>
              </div>
            </div>
          </template>
          <template v-else>{{ m.content }}</template>
        </div>
      </div>

      <!-- Kimi 风格底部输入栏 -->
      <div class="composer-kimi" :class="{ focused: composerFocused }">
        <!-- 引用块区（任务 C.2）-->
        <div v-if="quoteChips.length" class="quote-chips">
          <div
            v-for="q in quoteChips"
            :key="q.messageId"
            class="quote-chip"
            @click="scrollToMessage(q.messageId)"
          >
            <Icon name="quote" :size="14" />
            <span class="quote-chip-text">{{ q.snippet }}</span>
            <button
              type="button"
              class="quote-chip-close"
              title="移除引用"
              @click.stop="removeQuote(q.messageId)"
            >
              <Icon name="close" :size="14" />
            </button>
          </div>
        </div>

        <!-- composer-container 包裹层（任务 D.1）-->
        <div class="composer-container">
          <div ref="composerInnerRef" class="composer-inner">
            <IconButton
              size="md"
              container
              title="附件"
              @click="toolSheetOpen = true"
            >
              <Icon name="plus" :size="22" />
            </IconButton>
            <textarea
              ref="textareaRef"
              v-model="input"
              class="composer-input"
              :placeholder="sending ? '生成中…' : '尽管问，带图也行'"
              :disabled="sending"
              rows="1"
              @keydown="onKeydown"
              @focus="composerFocused = true"
              @blur="composerFocused = false"
              @input="autoResize"
            ></textarea>
            <Button
              v-if="!input.trim()"
              icon-only
              shape="circle"
              size="md"
              variant="normal"
              title="语音输入"
              @click="toast({ content: '语音输入即将上线', type: 'info' })"
            >
              <template #icon><Icon name="mic" :size="22" /></template>
            </Button>
            <Button
              v-else
              icon-only
              shape="circle"
              size="md"
              variant="primary"
              :disabled="!input.trim()"
              title="发送"
              @click="send"
            >
              <template #icon><Icon name="arrow-up" :size="22" /></template>
            </Button>
          </div>
        </div>

        <!-- 上下文 ring + 工作区（任务 D.7）-->
        <div class="composer-meta">
          <button
            type="button"
            class="meta-pill"
            title="上下文使用情况"
            @click="contextSheetOpen = true"
          >
            <ContextRing :used="contextUsedChars" :max="contextMaxChars" :size="18" />
            <span class="meta-pill-text">{{ contextUsedChars }} / {{ contextMaxChars }}</span>
          </button>
          <button
            type="button"
            class="meta-pill meta-pill--wd"
            :title="workingDir ?? '未设置'"
            @click="workingDirSheetOpen = true"
          >
            <Icon name="folder" :size="14" />
            <span class="meta-pill-text meta-pill-text--ellipsis">
              {{ workingDir ? workingDir : '默认工作区' }}
            </span>
          </button>
        </div>
      </div>
    </section>

    <!-- 消息长按 / 右键菜单（任务 C.1）-->
    <Menu
      v-model:visible="msgMenuVisible"
      :items="msgMenuItems"
      :position="msgMenuPosition"
      @select="onMsgMenuSelect"
    />

    <!-- 上下文管理 Sheet（任务 D.6）-->
    <BindSheet
      v-model:visible="contextSheetOpen"
      title="上下文管理"
      side="bottom"
      :height="'auto'"
    >
      <div class="ctx-sheet">
        <!-- 上下文使用统计 -->
        <div class="ctx-stat">
          <div class="ctx-stat-row">
            <ContextRing :used="contextUsedChars" :max="contextMaxChars" :size="32" />
            <div class="ctx-stat-text">
              <div class="ctx-stat-title">上下文使用</div>
              <div class="ctx-stat-desc">
                {{ contextUsedChars }} / {{ contextMaxChars }} 字符
                ·  约 {{ contextUsedTokens }} / {{ contextMaxTokens }} tokens
              </div>
            </div>
          </div>
        </div>

        <!-- 消息压缩按钮（任务 B 并行实现后端命令）-->
        <Button
          variant="primary"
          block
          :loading="compressing"
          :disabled="compressing"
          @click="triggerCompress"
        >
          <template #icon><Icon name="merge" :size="18" /></template>
          {{ compressing ? '压缩中…' : '压缩消息' }}
        </Button>

        <!-- 工作区显示（点击调出 workingDirSheet）-->
        <div
          class="tool-list-item"
          :title="workingDir ?? '未设置'"
          @click="contextSheetOpen = false; workingDirSheetOpen = true"
        >
          <span class="tool-list-icon"><Icon name="folder" :size="20" /></span>
          <div class="tool-list-text">
            <div class="tool-list-title">工作区</div>
            <div class="tool-list-desc">
              {{ workingDir ? workingDir : '未设置，相对路径以默认目录为准' }}
            </div>
          </div>
          <span class="tool-list-status">{{ workingDir ? '已设置' : '默认' }}</span>
          <span class="tool-list-arrow"><Icon name="chevron-right" :size="16" /></span>
        </div>

        <p class="ctx-hint">
          压缩消息会合并历史对话以释放上下文空间。工作区决定 read_file / list_files / shell 的相对路径基准。
        </p>
      </div>
    </BindSheet>

    <!-- 底部工具/附件 Sheet -->
    <BindSheet v-model:visible="toolSheetOpen" title="工具" side="bottom" :height="'auto'">
      <div class="tool-sheet">
        <div class="tool-row">
          <div
            v-for="t in toolCategories"
            :key="t.label"
            class="tool-card"
            @click="onToolClick(t.label)"
          >
            <span class="tool-card-icon"><Icon :name="t.icon" :size="24" /></span>
            <span class="tool-card-label">{{ t.label }}</span>
          </div>
        </div>

        <div class="tool-list">
          <div
            v-for="p in pluginItems"
            :key="p.label"
            class="tool-list-item"
            @click="onToolClick(p.label)"
          >
            <span class="tool-list-icon"><Icon :name="p.icon" :size="20" /></span>
            <div class="tool-list-text">
              <div class="tool-list-title">{{ p.label }}</div>
              <div class="tool-list-desc">{{ p.desc }}</div>
            </div>
            <span class="tool-list-arrow"><Icon name="chevron-right" :size="16" /></span>
          </div>
        </div>

        <div class="tool-list-item" @click="onToolClick('联网搜索')">
          <span class="tool-list-icon"><Icon name="globe" :size="20" /></span>
          <div class="tool-list-text">
            <div class="tool-list-title">联网搜索</div>
          </div>
          <span class="tool-list-status">自动</span>
          <span class="tool-list-arrow"><Icon name="chevron-right" :size="16" /></span>
        </div>

        <!-- 会话级工作区入口：read_file/list_files/shell 以此为基准 -->
        <div
          class="tool-list-item"
          :title="workingDir ? workingDir : '未设置，使用技能级或默认'"
          @click="workingDirSheetOpen = true"
        >
          <span class="tool-list-icon"><Icon name="folder" :size="20" /></span>
          <div class="tool-list-text">
            <div class="tool-list-title">工作区</div>
            <div class="tool-list-desc">
              {{ workingDir ? workingDir : '未设置，相对路径以默认目录为准' }}
            </div>
          </div>
          <span class="tool-list-status">{{ workingDir ? '已设置' : '默认' }}</span>
          <span class="tool-list-arrow"><Icon name="chevron-right" :size="16" /></span>
        </div>
      </div>
    </BindSheet>

    <!-- 工作区设置 Sheet：选择目录 / 清除 -->
    <BindSheet
      v-model:visible="workingDirSheetOpen"
      title="会话工作区"
      side="bottom"
      :height="'auto'"
    >
      <div class="wd-sheet">
        <div class="wd-current">
          <div class="wd-current-label">当前工作区</div>
          <div class="wd-current-path" :class="{ 'is-empty': !workingDir }">
            {{ workingDir || '未设置（使用技能级或进程默认目录）' }}
          </div>
        </div>
        <div class="wd-actions">
          <Button variant="primary" block @click="pickWorkingDir">
            <template #icon><Icon name="folder" :size="18" /></template>
            选择目录
          </Button>
          <Button
            variant="normal"
            block
            :disabled="!workingDir"
            @click="clearWorkingDir"
          >
            清除工作区
          </Button>
        </div>
        <p class="wd-hint">
          工作区决定 read_file / list_files / shell 的相对路径基准与命令执行目录。
          优先级：会话级 &gt; 技能级 &gt; 进程默认。
        </p>
      </div>
    </BindSheet>

    <!-- 图片全屏预览：Teleport 到 body，避免被 scoped 样式和层级影响 -->
    <Teleport to="body">
      <Transition name="img-preview-fade">
        <div
          v-if="previewState.visible"
          class="img-preview-overlay"
          @click="closeImagePreview"
        >
          <img
            :src="previewState.url"
            :alt="previewState.name"
            class="img-preview-img"
            @click.stop
          />
          <div class="img-preview-name">{{ previewState.name }}</div>
          <button
            type="button"
            class="img-preview-close"
            title="关闭（Esc）"
            @click.stop="closeImagePreview"
          >
            <Icon name="close" :size="22" />
          </button>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
/* ChatWindow 局部样式：补充基础组件未覆盖的部分 */
.composer-input {
  flex: 1;
  resize: none;
  min-height: 40px;
  max-height: 120px;
  padding: 10px 12px;
  font-family: inherit;
  font-size: 15px;
  color: var(--text);
  background: transparent;
  border: none;
  border-radius: var(--radius-full);
  outline: none;
  line-height: 1.4;
}

.composer-input::placeholder {
  color: var(--muted);
}

.composer-input:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

/* ---------- 任务 C：引用块 ---------- */
.quote-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: 0 4px;
}

.quote-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  padding: 4px 8px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
  /* contenteditable=false 防止误编辑 */
  user-select: none;
}

.quote-chip:hover {
  background: var(--border);
  border-color: var(--primary);
}

.quote-chip-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 220px;
}

.quote-chip-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  background: transparent;
  border: none;
  border-radius: 50%;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.quote-chip-close:hover {
  background: var(--danger);
  color: #fff;
}

/* ---------- 任务 D：composer 升级 ---------- */
/* focus 上抬：用 transform 避免 layout reflow，配合 transition 平滑 */
.composer-kimi {
  transition: transform 0.25s var(--ease-standard, ease);
}

.composer-kimi.focused {
  transform: translateY(-12px);
}

/* composer-container 包裹层：亮色 #CFCFCF，暗色用 --card-2 */
.composer-container {
  background: var(--card-2);
  border-radius: var(--radius-full);
  padding: 4px;
  transition: background var(--duration-fast) var(--ease-standard);
}

[data-theme='light'] .composer-container {
  background: #cfcfcf;
}

/* composer-inner 高度跟随 textarea；overflow hidden 防止超出时溢出 */
.composer-inner {
  overflow: hidden;
  max-height: 160px;
}

/* 上下文 ring + 工作区 meta 行 */
.composer-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 4px;
}

.meta-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  color: var(--muted);
  font-size: 12px;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.meta-pill:hover {
  background: var(--card-2);
  color: var(--text);
}

.meta-pill--wd {
  flex: 1;
  min-width: 0;
}

.meta-pill-text {
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

.meta-pill-text--ellipsis {
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ---------- 任务 D.6：上下文管理 Sheet ---------- */
.ctx-sheet {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 4px 0;
}

.ctx-stat {
  padding: 14px;
  border-radius: var(--radius-md, 12px);
  background: var(--card-2);
  border: 1px solid var(--border);
}

.ctx-stat-row {
  display: flex;
  align-items: center;
  gap: 14px;
}

.ctx-stat-text {
  flex: 1;
  min-width: 0;
}

.ctx-stat-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  margin-bottom: 4px;
}

.ctx-stat-desc {
  font-size: 12px;
  color: var(--muted);
  line-height: 1.5;
}

.ctx-hint {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted);
}

/* Kimi 风格空状态首页 */
.home-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px;
  gap: 48px;
  overflow-y: auto;
}

.home-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.home-logo {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 42px;
  font-weight: 800;
  letter-spacing: 2px;
  color: var(--text);
}

.home-logo-icon {
  font-size: 0.8em;
}

.home-subtitle {
  font-size: 16px;
  color: var(--text);
  font-weight: 500;
}

.home-subtitle.secondary {
  font-size: 15px;
  color: var(--muted);
  font-weight: 400;
}

.home-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 10px;
}

/* Kimi 风格底部工具 Sheet */
.tool-sheet {
  padding: 8px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.tool-row {
  display: flex;
  gap: 12px;
  overflow-x: auto;
  padding: 8px 0 16px;
}

.tool-card {
  flex: 0 0 auto;
  width: 88px;
  height: 88px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  background: var(--card-2);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.tool-card:hover {
  background: var(--border);
  transform: translateY(-2px);
}

.tool-card-icon {
  font-size: 28px;
  line-height: 1;
}

.tool-card-label {
  font-size: 13px;
  color: var(--text);
}

.tool-list {
  display: flex;
  flex-direction: column;
}

.tool-list-item {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 0;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.tool-list-item:last-child {
  border-bottom: none;
}

.tool-list-item:hover {
  background: var(--card-2);
  margin: 0 -20px;
  padding-left: 20px;
  padding-right: 20px;
}

.tool-list-icon {
  font-size: 22px;
  line-height: 1;
  width: 28px;
  text-align: center;
}

.tool-list-text {
  flex: 1;
  min-width: 0;
}

.tool-list-title {
  font-size: 15px;
  font-weight: 500;
  color: var(--text);
}

.tool-list-desc {
  font-size: 13px;
  color: var(--muted);
  margin-top: 2px;
}

.tool-list-status {
  font-size: 13px;
  color: var(--muted);
}

.tool-list-arrow {
  font-size: 18px;
  color: var(--muted);
}

/* 工作区设置 Sheet */
.wd-sheet {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 4px 0;
}

.wd-current {
  padding: 12px 14px;
  border-radius: 10px;
  background: var(--surface-alt, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--border);
}

.wd-current-label {
  font-size: 12px;
  color: var(--muted);
  margin-bottom: 6px;
}

.wd-current-path {
  font-size: 13px;
  line-height: 1.5;
  word-break: break-all;
  color: var(--text);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.wd-current-path.is-empty {
  color: var(--muted);
  font-style: italic;
}

.wd-actions {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.wd-hint {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted);
}

/* 消息内附件图片区域 */
.msg-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
}

.msg-attachment {
  position: relative;
  display: flex;
  flex-direction: column;
  border-radius: var(--radius-md, 12px);
  overflow: hidden;
  background: var(--card-2, rgba(0, 0, 0, 0.04));
  border: 1px solid var(--border);
  max-width: 320px;
}

.msg-attachment-img {
  display: block;
  max-width: 320px;
  max-height: 320px;
  width: auto;
  height: auto;
  object-fit: contain;
  cursor: zoom-in;
  transition: transform var(--duration-fast, 0.15s) var(--ease-standard, ease);
}

.msg-attachment-img:hover {
  transform: scale(1.02);
}

.msg-attachment-loading {
  width: 200px;
  height: 140px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--muted);
  font-size: 12px;
}

.msg-attachment-meta {
  padding: 6px 10px;
  font-size: 12px;
  color: var(--muted);
  background: var(--card-2, rgba(0, 0, 0, 0.02));
  border-top: 1px solid var(--border);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>

<!-- 非 scoped：图片预览遮罩通过 Teleport 渲染到 body，scoped 样式不会应用 -->
<style>
.img-preview-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  background: rgba(0, 0, 0, 0.88);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  padding: 32px;
  cursor: zoom-out;
}

.img-preview-img {
  max-width: 90vw;
  max-height: 80vh;
  object-fit: contain;
  border-radius: 8px;
  box-shadow: 0 12px 48px rgba(0, 0, 0, 0.5);
  cursor: default;
}

.img-preview-name {
  color: rgba(255, 255, 255, 0.85);
  font-size: 14px;
  font-weight: 500;
  max-width: 80vw;
  text-align: center;
  word-break: break-all;
}

.img-preview-close {
  position: absolute;
  top: 20px;
  right: 24px;
  width: 40px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.12);
  color: #fff;
  cursor: pointer;
  transition: background 0.15s ease;
}

.img-preview-close:hover {
  background: rgba(255, 255, 255, 0.22);
}

.img-preview-fade-enter-active,
.img-preview-fade-leave-active {
  transition: opacity 0.2s ease;
}

.img-preview-fade-enter-from,
.img-preview-fade-leave-to {
  opacity: 0;
}
</style>
