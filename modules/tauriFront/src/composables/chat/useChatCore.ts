/**
 * 聊天核心状态与会话生命周期
 *
 * 职责:会话级状态(activeId / messages / input / sending / workingDir /
 * activeModelInfo)、会话加载与切换、上下文使用统计、工作区管理、
 * 通用工具函数。
 *
 * 跨领域协作通过 setSessionHooks 注入:
 *  - resetAll : 会话切换/清空时,由 ChatWindow 组合各子 store 清空 UI 状态
 *  - afterLoad: 历史消息加载成功后,恢复气泡元数据 / 附件 / 任务清单
 */
import { ref, computed, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useTheme } from '../useTheme'
import { useToast } from '../../components/basic'
import { NEW_CHAT_TAB_ID } from '../useTabs'
import type { Conversation, Message, PickedFile } from '../../types'
import type { useAutoScroll } from './useAutoScroll'


/** 模块级消息 id 计数器（与后端 gen_message_id() 同布局：高 42 位毫秒时间戳 + 低 22 位递增计数器）。
 *  前端仅在流式/本地气泡期间临时使用（落盘后以后端持久化 id 为准），统一为短数字字符串。 */
let idSeq = 0
/** 当前激活模型信息(含计费单价),用于上下文窗口大小 + 历史计费重算 */
export interface ActiveModelInfo {
  id: string
  name: string
  context_window_tokens: number | null
  /** 当前激活模型的计费单价(元/百万 tokens);未配置时为 null */
  pricing?: {
    cache_hit_per_m: number
    cache_miss_per_m: number
    output_per_m: number
  } | null
}

/** 会话级生命周期钩子(由 ChatWindow 组合各子 store 注入) */
export interface SessionHooks {
  /** 会话切换/清空时:清空所有依赖会话的 UI 状态 */
  resetAll: () => void
  /** 历史消息加载成功后:恢复气泡元数据 / 附件 / 任务清单(异步) */
  afterLoad: () => Promise<void>
}

export interface ChatCoreProps {
  backend?: string
  conversationId?: string | null
}

export interface ChatCoreEmits {
  (e: 'update:conversation-id', id: string | null): void
  (e: 'conversation-changed'): void
}

/** 上下文使用统计:粗略 4 字符 = 1 token */
const fallbackContextTokens = 128000

export function useChatCore(
  props: ChatCoreProps,
  emit: ChatCoreEmits,
  autoscroll: ReturnType<typeof useAutoScroll>,
) {
  const { resolvedTheme } = useTheme()
  const { toast } = useToast()

  const isDark = computed(() => resolvedTheme.value === 'dark')

  // ---------- 会话级状态 ----------
  // activeId:当前正在交互的会话 id(流式事件匹配用)。
  // 与 props.conversationId 同步,但在新建会话时可立即赋值,不等 App 回传。
  const activeId = ref<string | null>(props.conversationId ?? null)
  const messages = ref<Message[]>([])
  const input = ref('')
    const sending = ref(false)
    /** 生成期间排队待插入的用户消息条数（AI 生成中仍可发送，在下一 completion 前插入） */
    const queuedCount = ref(0)
    /** 会话级工作区路径:None 表示未设置(回退到技能级或进程默认) */
    const workingDir = ref<string | null>(null)
  const workingDirSheetOpen = ref(false)
  /** 底部工具/附件 Sheet(由 composer 打开,ToolSheet 渲染) */
  const toolSheetOpen = ref(false)
  /** 命令会话便签栏(ShellSessionBar)折叠状态:由 composer-meta 按钮控制 */
  const shellBarExpanded = ref(false)
  /** 正在运行的命令会话数(ShellSessionBar 上报,供 composer-meta 徽标展示) */
  const shellActiveCount = ref(0)
  function toggleShellBar() {
    shellBarExpanded.value = !shellBarExpanded.value
  }

  /** 上下文管理 Sheet(含消息压缩按钮) */
  const contextSheetOpen = ref(false)
  /** 右栏上下文面板:默认开启,可从底栏 meta 区域切换 */
  const ctxPanelOpen = ref(true)

  const activeModelInfo = ref<ActiveModelInfo | null>(null)
  async function loadActiveModelInfo() {
    try {
      activeModelInfo.value = await invoke<ActiveModelInfo>('get_active_model_info')
    } catch {
      activeModelInfo.value = null
    }
  }

  // ---------- 上下文使用统计 ----------
  // 上下文真实占用以 API usage 的 prompt_tokens 为准（agent-usage.input_tokens）：
  // token 是模型分词器的真实计数，不能靠 字符串.length/4 估算；且 prompt_tokens
  // 天然包含注入的思维链（（思考：...））、系统提示、记忆等整段 prompt。
  // 流式期间由 agent-usage 事件实时更新；会话加载时从历史消息 usage 恢复；
  // 均无真实数据（如未记录 usage 的旧会话）才回退为字符估算兜底。
  const realContextUsedTokens = ref<number | null>(null)

  const contextMaxTokens = computed(() =>
    activeModelInfo.value?.context_window_tokens ?? fallbackContextTokens,
  )
  const contextUsedChars = computed(() =>
    messages.value.reduce((sum, m) => {
      let chars = m.content?.length ?? 0
      // 字符统计仅作兜底/参考展示，不作为上下文占用口径：
      if (m.reasoning) chars += m.reasoning.length
      return sum + chars
    }, 0),
  )
  const contextUsedTokens = computed(() =>
    realContextUsedTokens.value ?? Math.ceil(contextUsedChars.value / 4),
  )

  /** 用真实 token 占用更新上下文使用仪表盘（agent-usage 事件 / 历史恢复共用） */
  function setContextUsedTokens(tokens: number | null) {
    realContextUsedTokens.value = tokens
  }

  function toggleCtxPanel() {
    ctxPanelOpen.value = !ctxPanelOpen.value
  }

  // ---------- 会话加载 ----------
  let hooks: SessionHooks = { resetAll: () => {}, afterLoad: async () => {} }
  function setSessionHooks(h: SessionHooks) {
    hooks = h
  }

  async function loadConversation() {
    const id = activeId.value
    if (!id) {
      messages.value = []
      workingDir.value = null
      realContextUsedTokens.value = null
      hooks.resetAll()
      return
    }
    try {
      // 先确保已加载激活模型信息(含计费单价),供历史消息计费重算
      await loadActiveModelInfo()
      const conv = await invoke<Conversation | null>('get_conversation', { id })
      messages.value = conv?.messages ?? []
      workingDir.value = conv?.working_dir ?? null
      // 从历史消息恢复上下文真实占用：取最后一条带 usage 的 assistant 消息的
      // input_tokens（该消息生成时的 prompt_tokens 即当时的真实上下文占用）
      let realTokens: number | null = null
      for (let i = messages.value.length - 1; i >= 0; i--) {
        const u = messages.value[i].usage
        if (u && u.input_tokens > 0) {
          realTokens = u.input_tokens
          break
        }
      }
      realContextUsedTokens.value = realTokens
      // 切换会话:清空上一会话的 meta / 引用 / 任务模式,再恢复新会话元数据
      hooks.resetAll()
      await hooks.afterLoad()
      // 进入新会话:强制 sticky + pin 一段时间持续跟随底部
      // markstream-vue 异步渲染 / 附件 base64 / 图片解码会持续改变 scrollHeight,
      // 单次 nextTick + scrollBottom 会因 DOM 还在增长而停在中间位置
      autoscroll.stickToBottom.value = true
      await nextTick()
      autoscroll.jumpToBottom()
      // pin 窗口覆盖 markstream-vue 异步渲染 + 图片解码 + 附件回填的延迟增长
      autoscroll.pinToBottom(800)
    } catch (e) {
      console.warn('get_conversation failed', e)
      messages.value = []
      workingDir.value = null
      realContextUsedTokens.value = null
      hooks.resetAll()
    }
  }

  /** 设置当前会话 id(由 App 回传时同步) */
  function setActiveId(id: string | null) {
    activeId.value = id
  }

  // 优先级:会话级 > 技能级(apply_skill 写入) > 进程默认 cwd
  // 新建对话页签尚无真实会话(conversationId 为 null 或 __new_chat__ 哨兵):
  // 需要真实会话 id 的操作(设置工作区 / 发消息)先经此创建会话并回传真实 id,
  // 同时触发页签迁移(TabContent 将 __new_chat__ 迁移为真实 conversation_id)。
  async function ensureConversation(): Promise<string | null> {
    let id = activeId.value
    if (id && id !== NEW_CHAT_TAB_ID) return id
    try {
      id = await invoke<string>('create_conversation')
      activeId.value = id
      emit('update:conversation-id', id)
      emit('conversation-changed')
      return id
    } catch (e) {
      toast({ content: `新建会话失败：${e}`, type: 'error' })
      return null
    }
  }

  async function pickWorkingDir() {
    // 先选目录再建会话:用户取消选择时不会凭空产生一个空会话
    let path: string | null = null
    try {
      path = await invoke<string | null>('pick_directory')
    } catch (e) {
      toast({ content: `选择目录失败：${e}`, type: 'error' })
      return
    }
    if (!path) return
    const id = await ensureConversation()
    if (!id) return
    try {
      await invoke('set_conversation_working_dir', {
        conversationId: id,
        workingDir: path,
      })
      workingDir.value = path
      toast({ content: `已设置工作区：${path}`, type: 'success' })
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

  // ---------- 通用工具 ----------
    function newId(): string {
      const now = BigInt(Date.now())
      const seq = BigInt(idSeq++ & 0x3fffff)
      return ((now << 22n) | seq).toString()
    }

  function formatFileSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  }

  // ---------- 空状态 ----------
  const isEmptyHome = computed(() => messages.value.length === 0 && !sending.value)

  return {
    props,
    emit,
    isDark,
    toast,
      activeId,
      messages,
      input,
      sending,
        queuedCount,
      workingDir,
      workingDirSheetOpen,
      toolSheetOpen,
    shellBarExpanded,
    shellActiveCount,
    toggleShellBar,
    contextSheetOpen,
    ctxPanelOpen,
    activeModelInfo,
    contextMaxTokens,
    contextUsedChars,
    contextUsedTokens,
    setContextUsedTokens,
    toggleCtxPanel,
    setSessionHooks,
    loadConversation,
    loadActiveModelInfo,
    setActiveId,
    ensureConversation,
    pickWorkingDir,
    clearWorkingDir,
    newId,
      formatFileSize,
      isEmptyHome,
    }
  }
