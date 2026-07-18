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
  AgentUsagePayload,
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

// ---------- msg-bubble 高度动画 ----------
// 设计说明：早期版本曾用 ResizeObserver + offsetHeight 实现 FLIP 高度动画，
// 但在流式追加内容场景下会"抽搐"：
//   - ResizeObserver 回调中读 offsetHeight 触发强制 reflow，且读到的是被
//     height 样式锁定后的高度，不是内容自然高度
//   - 锁定 height 会再次触发自身 observer，形成竞态
//   - animate 期间内容又增长，结束时清空 height 会瞬间跳变
// FLIP 适合离散状态切换（展开/折叠），不适合流式追加。
// 因此这里完全移除高度动画，仅保留 spawn 时的 opacity + scale 过渡，
// 内容增长交给浏览器原生 reflow + markstream-vue 的 smooth-streaming 处理。

// 每个助手气泡的元数据：reasoning / tool calls / usage（流式期间累积，不持久化）
interface BubbleMeta {
  reasoning: string
  isThinking: boolean
  toolCalls: ToolCallRecord[]
  /** token 使用统计：仅在 agent-usage 事件到达后赋值，流式结束保留显示 */
  usage: AgentUsagePayload | null
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
      usage: null,
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
  // 同步清理 bubbleMeta / quoteChips
  delete bubbleMeta[id]
  quoteChips.value = quoteChips.value.filter((q) => q.messageId !== id)
  toast({ content: '已从视图移除', type: 'info' })
}

// ---------- composer 升级（任务 D）----------
const composerFocused = ref(false)
const textareaRef = ref<HTMLTextAreaElement | null>(null)
const composerInnerRef = ref<HTMLElement | null>(null)

// 当前激活模型信息（用于真实上下文窗口大小）
interface ActiveModelInfo {
  id: string
  name: string
  context_window_tokens: number | null
}

const activeModelInfo = ref<ActiveModelInfo | null>(null)

async function loadActiveModelInfo() {
  try {
    activeModelInfo.value = await invoke<ActiveModelInfo>('get_active_model_info')
  } catch {
    activeModelInfo.value = null
  }
}

// 上下文使用统计：粗略 4 字符 = 1 token
const fallbackContextTokens = 128000
const contextMaxTokens = computed(() =>
  activeModelInfo.value?.context_window_tokens ?? fallbackContextTokens,
)
const contextUsedChars = computed(() =>
  messages.value.reduce((sum, m) => sum + (m.content?.length ?? 0), 0),
)
const contextUsedTokens = computed(() => Math.ceil(contextUsedChars.value / 4))

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
    height:  targetHeight + 'px',
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
    // 清空 meta / 附件缓存
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
    // 切换会话时清空附件缓存，避免上一会话残留
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
// bubble spawn 动画：仅 opacity + scale，不操作 height。
// 高度变化交给浏览器原生 reflow + markstream-vue 的 smooth-streaming 处理，
// 避免 ResizeObserver + offsetHeight 在流式场景下的抽搐问题。
async function addMessage(msg: Message) {
  messages.value.push(msg)
  await nextTick()
  const el = document.getElementById('msg-' + msg.id)
  if (!el) {
    scrollBottom()
    return
  }

  // 初始状态：透明 + 缩放 0.96（轻微，避免大幅缩放导致内容模糊）
  // 不设置 height/overflow，让内容自然撑开
  el.style.opacity = '0'
  el.style.transform = 'scale(0.96)'
  el.style.transformOrigin = 'center top'
  // 强制 reflow 确保 anime.js 起点准确
  void el.offsetHeight

  animate(el, {
    opacity: [0, 1],
    scale: [0.96, 1],
    duration: 280,
    ease: 'out(3)',
    onComplete: () => {
      el.style.opacity = ''
      el.style.transform = ''
      el.style.transformOrigin = ''
    },
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

// ---------- token 使用统计 ----------
// agent-usage 事件在每次 CompletionCall 结束时 emit 一次：
//   - 单次值（input_tokens/output_tokens/total_tokens/reasoning_tokens）：本次 completion
//   - 累计值（cumulative_*）：本轮会话所有 completion 的累加
// 写入"当前 streamingBubbleId"对应的 meta.usage；若介于两段 completion 之间
// （streamingBubbleId 已被清空但新 token 未到达），回退到最近一条 assistant 消息。
async function onUsage(p: AgentUsagePayload) {
  let targetId = streamingBubbleId.value
  if (!targetId) {
    // 回退：找最后一条 assistant 消息
    for (let i = messages.value.length - 1; i >= 0; i--) {
      if (messages.value[i].role === 'assistant') {
        targetId = messages.value[i].id
        break
      }
    }
  }
  if (!targetId) return
  const meta = ensureMeta(targetId)
  meta.usage = p
}

// ---------- 图片预览 ----------
// 点击消息内图片打开全屏预览（Teleport 到 body）。
// 支持缩放（滚轮/按钮）、平移（拖拽）、旋转（按钮）、双击复位。
// Esc 关闭，再次点击遮罩关闭。
const previewState = reactive({
  visible: false,
  url: '',
  name: '',
  scale: 1,
  rotate: 0,
  tx: 0,
  ty: 0,
})
// 拖拽状态：pointerdown 记录起点，pointermove 更新 tx/ty
const previewDrag = reactive({ active: false, startX: 0, startY: 0, baseTx: 0, baseTy: 0 })

function resetPreviewTransform() {
  previewState.scale = 1
  previewState.rotate = 0
  previewState.tx = 0
  previewState.ty = 0
}

function openImagePreview(url: string, name: string) {
  if (!url) return
  previewState.url = url
  previewState.name = name
  resetPreviewTransform()
  previewState.visible = true
}

function closeImagePreview() {
  previewState.visible = false
  previewState.url = ''
  previewState.name = ''
  resetPreviewTransform()
}

function previewZoomIn() {
  previewState.scale = Math.min(previewState.scale * 1.25, 8)
}
function previewZoomOut() {
  previewState.scale = Math.max(previewState.scale / 1.25, 0.2)
}
function previewRotate() {
  previewState.rotate = (previewState.rotate + 90) % 360
}

function onPreviewWheel(e: WheelEvent) {
  e.preventDefault()
  if (e.deltaY < 0) previewZoomIn()
  else previewZoomOut()
}

function onPreviewPointerDown(e: PointerEvent) {
  // 仅主键（左键 / 触摸）触发拖拽
  if (e.button !== 0 && e.pointerType === 'mouse') return
  previewDrag.active = true
  previewDrag.startX = e.clientX
  previewDrag.startY = e.clientY
  previewDrag.baseTx = previewState.tx
  previewDrag.baseTy = previewState.ty
  ;(e.target as HTMLElement).setPointerCapture?.(e.pointerId)
}

function onPreviewPointerMove(e: PointerEvent) {
  if (!previewDrag.active) return
  previewState.tx = previewDrag.baseTx + (e.clientX - previewDrag.startX)
  previewState.ty = previewDrag.baseTy + (e.clientY - previewDrag.startY)
}

function onPreviewPointerUp(e: PointerEvent) {
  previewDrag.active = false
  ;(e.target as HTMLElement).releasePointerCapture?.(e.pointerId)
}

function onPreviewDblClick() {
  // 双击复位缩放与平移（保留旋转）
  previewState.scale = 1
  previewState.tx = 0
  previewState.ty = 0
}

function onPreviewKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') closeImagePreview()
  else if (e.key === '+' || e.key === '=') previewZoomIn()
  else if (e.key === '-') previewZoomOut()
  else if (e.key === '0') { previewState.scale = 1; previewState.tx = 0; previewState.ty = 0 }
  else if (e.key === 'r') previewRotate()
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

  // 获取当前激活模型信息，backend 切换时重新加载
  await loadActiveModelInfo()
  watch(
    () => props.backend,
    () => loadActiveModelInfo(),
    { immediate: false },
  )

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
    await listen<AgentUsagePayload>('agent-usage', async (e) => {
      const p = e.payload
      if (activeId.value && p.conversation_id !== activeId.value) return
      await onUsage(p)
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
              :smooth-streaming="false"
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
            <!-- token 使用统计：每次 CompletionCall 结束后实时显示 -->
            <div
              v-if="getMeta(m.id)?.usage"
              class="msg-usage"
              :class="{ streaming: m.id === streamingBubbleId }"
              :title="`本次：输入 ${getMeta(m.id)!.usage!.input_tokens} · 输出 ${getMeta(m.id)!.usage!.output_tokens}${getMeta(m.id)!.usage!.reasoning_tokens > 0 ? ' · 推理 ' + getMeta(m.id)!.usage!.reasoning_tokens : ''}\n累计：输入 ${getMeta(m.id)!.usage!.cumulative_input} · 输出 ${getMeta(m.id)!.usage!.cumulative_output} · 合计 ${getMeta(m.id)!.usage!.cumulative_total}`"
            >
              <span class="usage-label">tokens</span>
              <span class="usage-val">{{ getMeta(m.id)!.usage!.input_tokens }}</span>
              <span class="usage-sep">/</span>
              <span class="usage-val">{{ getMeta(m.id)!.usage!.output_tokens }}</span>
              <span
                v-if="getMeta(m.id)!.usage!.reasoning_tokens > 0"
                class="usage-val usage-reasoning"
              >+{{ getMeta(m.id)!.usage!.reasoning_tokens }}</span>
              <span class="usage-sep">·</span>
              <span class="usage-cumulative">累计 {{ getMeta(m.id)!.usage!.cumulative_total }}</span>
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
                  <!-- 上下文 ring + 工作区（任务 D.7）-->
        <div class="composer-meta">
          <button
            type="button"
            class="meta-pill meta-pill--context"
            :title="`上下文使用：${contextUsedTokens} / ${contextMaxTokens} tokens`"
            @click="contextSheetOpen = true"
          >
            <ContextRing :used="contextUsedTokens" :max="contextMaxTokens" :size="18" />
            <span class="usage-label">tokens</span>
            <span class="usage-val">{{ contextUsedTokens }}</span>
            <span class="usage-sep">/</span>
            <span class="usage-val">{{ contextMaxTokens }}</span>
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
            <ContextRing :used="contextUsedTokens" :max="contextMaxTokens" :size="32" />
            <div class="ctx-stat-text">
              <div class="ctx-stat-title">上下文使用</div>
              <div class="ctx-stat-desc">
                {{ contextUsedTokens }} / {{ contextMaxTokens }} tokens
                · 约 {{ contextUsedChars }} 字符
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
    <!-- 支持滚轮缩放、拖拽平移、按钮旋转、双击复位、Esc 关闭 -->
    <Teleport to="body">
      <Transition name="img-preview-fade">
        <div
          v-if="previewState.visible"
          class="img-preview-overlay"
          @click="closeImagePreview"
          @wheel="onPreviewWheel"
        >
          <img
            :src="previewState.url"
            :alt="previewState.name"
            class="img-preview-img"
            :style="{ transform: `translate(${previewState.tx}px, ${previewState.ty}px) scale(${previewState.scale}) rotate(${previewState.rotate}deg)` }"
            @click.stop
            @pointerdown="onPreviewPointerDown"
            @pointermove="onPreviewPointerMove"
            @pointerup="onPreviewPointerUp"
            @pointercancel="onPreviewPointerUp"
            @dblclick="onPreviewDblClick"
            draggable="false"
          />
          <div class="img-preview-name">{{ previewState.name }}</div>

          <!-- 工具栏：放大 / 缩小 / 旋转 / 复位 / 关闭 -->
          <div class="img-preview-toolbar" @click.stop>
            <button type="button" class="img-preview-tool-btn" title="放大（+）" @click="previewZoomIn">
              <Icon name="plus" :size="20" />
            </button>
            <span class="img-preview-zoom-label">{{ Math.round(previewState.scale * 100) }}%</span>
            <button type="button" class="img-preview-tool-btn" title="缩小（-）" @click="previewZoomOut">
              <Icon name="minus" :size="20" />
            </button>
            <span class="img-preview-tool-divider"></span>
            <button type="button" class="img-preview-tool-btn" title="旋转 90°（R）" @click="previewRotate">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M4 12C4 7.6 7.6 4 12 4C14.5 4 16.7 5.1 18.2 6.9M20 4V9H15M20 12C20 16.4 16.4 20 12 20C9.5 20 7.3 18.9 5.8 17.1M4 20V15H9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
            <button type="button" class="img-preview-tool-btn" title="复位（0）" @click="resetPreviewTransform">
              <Icon name="refresh" :size="20" />
            </button>
            <span class="img-preview-tool-divider"></span>
            <button type="button" class="img-preview-tool-btn img-preview-tool-btn--close" title="关闭（Esc）" @click="closeImagePreview">
              <Icon name="close" :size="20" />
            </button>
          </div>
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
  border-radius: 32px;
  padding: 5px;
  transition: background var(--duration-fast) var(--ease-standard);
}

[data-theme='light'] .composer-container {
  background: #eeeeee;
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
  padding: 4px 4px;
  /* margin-bottom: 5px; */
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
  width: fit-content;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.meta-pill:hover {
  background: var(--card-2);
  color: var(--text);
}

.meta-pill--wd {
  /* flex: 1; */
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

/* 底部上下文 pill：视觉风格与 .msg-usage 完全一致 */
.meta-pill--context {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: var(--card-2);
  font-size: var(--fs-xs, 12px);
  line-height: 1.5;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.meta-pill--context:hover {
  background: var(--card-2);
  color: var(--muted);
}

.meta-pill--context .usage-label,
.meta-pill--context .usage-sep,
.meta-pill--context .usage-val {
  font-family: inherit;
}

.meta-pill--context .usage-label {
  color: var(--muted);
  font-weight: 500;
}

.meta-pill--context .usage-val {
  color: var(--text);
  font-weight: 500;
}

.meta-pill--context .usage-sep {
  color: var(--muted);
  opacity: 0.6;
}

/* ---------- 任务 D.6：上下文管理 Sheet ---------- */
.ctx-sheet {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px;
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

/* token 使用统计：assistant 气泡底部小标签
 * 视觉风格对齐 .meta-pill / .ctx-stat-desc：
 * - 中性 var(--muted) 颜色，无彩色箭头
 * - 数字使用 SFMono-Regular + tabular-nums，与 char-badge / stat-value 一致
 * - · 作为分隔符，与 ctx-stat-desc（"X / Y 字符 · 约 X / Y tokens"）保持一致
 * - 边框使用 var(--border) 实线（不用 dashed），与 .meta-pill 一致
 */
.msg-usage {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-top: 8px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: var(--card-2);
  font-size: var(--fs-xs, 12px);
  line-height: 1.5;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  font-family: 'SFMono-Regular', Consolas, monospace;
  user-select: none;
  width: fit-content;
  max-width: 100%;
}

.msg-usage.streaming {
  opacity: 0.8;
}

.msg-usage .usage-label {
  color: var(--muted);
  font-family: inherit;
  font-weight: 500;
}

.msg-usage .usage-val {
  color: var(--text);
  font-weight: 500;
}

.msg-usage .usage-reasoning {
  color: var(--muted);
}

.msg-usage .usage-sep {
  color: var(--muted);
  opacity: 0.6;
}

.msg-usage .usage-cumulative {
  color: var(--muted);
  font-family: inherit;
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
  user-select: none;
}

.img-preview-img {
  max-width: 90vw;
  max-height: 80vh;
  object-fit: contain;
  border-radius: 8px;
  box-shadow: 0 12px 48px rgba(0, 0, 0, 0.5);
  cursor: grab;
  /* transform 由 Vue 内联 style 控制；transition 让缩放/旋转/平移平滑 */
  transition: transform 0.15s ease-out;
  will-change: transform;
  touch-action: none; /* 让 pointer 事件不被浏览器手势抢占 */
}

.img-preview-img:active {
  cursor: grabbing;
}

.img-preview-name {
  color: rgba(255, 255, 255, 0.85);
  font-size: 14px;
  font-weight: 500;
  max-width: 80vw;
  text-align: center;
  word-break: break-all;
}

/* 工具栏：放大 / 缩小 / 旋转 / 复位 / 关闭 */
.img-preview-toolbar {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 10px;
  background: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(12px);
  border-radius: 999px;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.img-preview-tool-btn {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: #fff;
  cursor: pointer;
  transition: background 0.15s ease;
  padding: 0;
}

.img-preview-tool-btn:hover {
  background: rgba(255, 255, 255, 0.16);
}

.img-preview-tool-btn--close:hover {
  background: rgba(232, 65, 65, 0.5);
}

.img-preview-zoom-label {
  color: rgba(255, 255, 255, 0.9);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  min-width: 48px;
  text-align: center;
  user-select: none;
}

.img-preview-tool-divider {
  width: 1px;
  height: 20px;
  background: rgba(255, 255, 255, 0.18);
  margin: 0 4px;
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
