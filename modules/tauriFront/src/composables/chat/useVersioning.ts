/**
 * useVersioning —— 会话版本控制（git 风格）组合式
 *
 * # 职责
 * - 封装后端 version_* 系列 Tauri 命令（list / create_branch / save_temp /
 *   rollback / undo_before / checkout / delete_ref）
 * - 持有响应式版本列表（分支 / 临时版本 / 检查点 / 当前分支提交链）
 * - 消息气泡 hover 操作：复制 / 开启分支 / 保存临时版本 / 回溯版本 / 撤回至此消息前
 * - 破坏性操作（回溯/撤回/检出）先弹确认框，成功后重载消息区 + 刷新版本列表
 *
 * # 设计
 * - 实例级状态：每个 ChatWindow（会话页签）一份，多实例互不污染
 * - 所有破坏性操作后端都会自动保存 chkpt-* 检查点，可随时找回
 */
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type {
  Message,
  VersionList,
  VersionOpResult,
  VersionRefSummary,
} from '../../types'
import type { useChatCore } from './useChatCore'

/** 确认框状态 */
export interface VersionConfirmState {
  visible: boolean
  title: string
  content: string
  confirmText: string
  danger: boolean
  onConfirm: () => void
}

export function useVersioning(core: ReturnType<typeof useChatCore>) {
  /** 会话版本列表（null = 无会话或未加载） */
  const list = ref<VersionList | null>(null)
  const loading = ref(false)
  /** 版本管理面板开关 */
  const sheetOpen = ref(false)
  /** 破坏性操作确认框 */
  const confirmState = ref<VersionConfirmState | null>(null)

  function requireConv(): string | null {
    const id = core.activeId.value
    if (!id || id.startsWith('__')) {
      core.toast({ content: '当前没有可用会话', type: 'info' })
      return null
    }
    return id
  }

  /** 刷新会话版本列表（无会话时清空） */
  async function loadVersions() {
    const id = core.activeId.value
    if (!id || id.startsWith('__')) {
      list.value = null
      return
    }
    loading.value = true
    try {
      list.value = await invoke<VersionList>('version_list', { conversationId: id })
    } catch {
      list.value = null
    } finally {
      loading.value = false
    }
  }

  /** 版本操作成功的统一收尾：重载消息 + 刷新版本列表 + toast */
  async function afterOp(result: VersionOpResult | null, msg: string) {
    if (!result) return
    await core.loadConversation()
    await loadVersions()
    core.toast({ content: msg, type: 'success' })
  }

  /** 计算删除该消息（含/不含）后将被移除的消息数 */
  function removedCount(m: Message, inclusive: boolean): number {
    const idx = core.messages.value.findIndex((x) => x.id === m.id)
    if (idx < 0) return 0
    return core.messages.value.length - idx - (inclusive ? 0 : 1)
  }

  // ---------- 气泡 hover 操作 ----------

  /** 复制信息 */
  async function onCopy(m: Message) {
    try {
      await navigator.clipboard.writeText(m.content)
      core.toast({ content: '已复制', type: 'success' })
    } catch (e) {
      core.toast({ content: `复制失败：${e}`, type: 'error' })
    }
  }

  /** 开启分支：从该消息点开新分支，其后消息留在原分支 */
  async function onBranch(m: Message) {
    const id = requireConv()
    if (!id) return
    loading.value = true
    try {
      const r = await invoke<VersionOpResult>('version_create_branch', {
        conversationId: id,
        messageId: m.id,
      })
      await afterOp(r, `已开启分支「${r.branch}」，对话从此消息继续`)
    } catch (e) {
      core.toast({ content: `开启分支失败：${e}`, type: 'error' })
    } finally {
      loading.value = false
    }
  }

  /** 保存临时版本：在该消息点打书签（不移动对话） */
  async function onSaveTemp(m: Message) {
    const id = requireConv()
    if (!id) return
    loading.value = true
    try {
      const note = (m.content || '').replace(/\s+/g, ' ').slice(0, 30) || '临时版本'
      const r = await invoke<VersionRefSummary>('version_save_temp', {
        conversationId: id,
        messageId: m.id,
        note,
      })
      await loadVersions()
      core.toast({ content: `已保存临时版本「${r.name}」`, type: 'success' })
    } catch (e) {
      core.toast({ content: `保存临时版本失败：${e}`, type: 'error' })
    } finally {
      loading.value = false
    }
  }

  /** 回溯版本：重置对话到此消息（丢弃其后）—— 破坏性，先确认 */
  function onRollback(m: Message) {
    const n = removedCount(m, false)
    confirmState.value = {
      visible: true,
      title: '回溯版本',
      content: `将把对话回溯到「${snippet(m)}」这条消息，其后的 ${n} 条消息将从当前分支移除。\n当前状态会自动保存为检查点，随时可在「版本管理」中找回。`,
      confirmText: '回溯',
      danger: true,
      onConfirm: () => void doRollback(m),
    }
  }

  async function doRollback(m: Message) {
    const id = requireConv()
    if (!id) return
    loading.value = true
    try {
      const r = await invoke<VersionOpResult>('version_rollback', {
        conversationId: id,
        messageId: m.id,
      })
      await afterOp(r, `已回溯到消息版本（当前 ${r.messages.length} 条消息）`)
    } catch (e) {
      core.toast({ content: `回溯失败：${e}`, type: 'error' })
    } finally {
      loading.value = false
    }
  }

  /** 撤回至此消息前：删除该消息及其后全部 —— 破坏性，先确认 */
  function onUndoBefore(m: Message) {
    const n = removedCount(m, true)
    confirmState.value = {
      visible: true,
      title: '撤回至此消息前',
      content: `将撤回「${snippet(m)}」这条消息及其后的全部 ${n} 条消息，对话回到它之前的状态。\n当前状态会自动保存为检查点，随时可在「版本管理」中找回。`,
      confirmText: '撤回',
      danger: true,
      onConfirm: () => void doUndoBefore(m),
    }
  }

  async function doUndoBefore(m: Message) {
    const id = requireConv()
    if (!id) return
    loading.value = true
    try {
      const r = await invoke<VersionOpResult>('version_undo_before', {
        conversationId: id,
        messageId: m.id,
      })
      await afterOp(r, `已撤回至此消息前（当前 ${r.messages.length} 条消息）`)
    } catch (e) {
      core.toast({ content: `撤回失败：${e}`, type: 'error' })
    } finally {
      loading.value = false
    }
  }

  // ---------- 版本管理面板操作 ----------

  /** 检出到引用（分支切换 / 临时版本或检查点 → 新建分支继续） */
  function onCheckout(refName: string) {
    confirmState.value = {
      visible: true,
      title: '检出版本',
      content: `将切换到版本「${refName}」，消息区恢复为该版本的内容。当前状态会自动保存为检查点。`,
      confirmText: '检出',
      danger: false,
      onConfirm: () => void doCheckout(refName),
    }
  }

  async function doCheckout(refName: string) {
    const id = requireConv()
    if (!id) return
    loading.value = true
    try {
      const r = await invoke<VersionOpResult>('version_checkout', {
        conversationId: id,
        refName,
      })
      await afterOp(r, `已检出「${refName}」`)
    } catch (e) {
      core.toast({ content: `检出失败：${e}`, type: 'error' })
    } finally {
      loading.value = false
    }
  }

  /** 删除引用（临时版本 / 检查点 / 分支） */
  function onDeleteRef(refName: string) {
    confirmState.value = {
      visible: true,
      title: '删除版本引用',
      content: `将删除引用「${refName}」。此操作不会影响消息内容，仅移除该版本书签。`,
      confirmText: '删除',
      danger: true,
      onConfirm: () => void doDeleteRef(refName),
    }
  }

  async function doDeleteRef(refName: string) {
    const id = requireConv()
    if (!id) return
    loading.value = true
    try {
      await invoke('version_delete_ref', { conversationId: id, refName })
      await loadVersions()
      core.toast({ content: `已删除引用「${refName}」`, type: 'success' })
    } catch (e) {
      core.toast({ content: `删除失败：${e}`, type: 'error' })
    } finally {
      loading.value = false
    }
  }

  function closeConfirm() {
    confirmState.value = null
  }

  return {
    list,
    loading,
    sheetOpen,
    confirmState,
    loadVersions,
    onCopy,
    onBranch,
    onSaveTemp,
    onRollback,
    onUndoBefore,
    onCheckout,
    onDeleteRef,
    closeConfirm,
  }
}

/** 消息摘要（前 24 字符） */
function snippet(m: Message): string {
  const t = (m.content || '').trim().replace(/\s+/g, ' ')
  return t.length <= 24 ? t : t.slice(0, 24) + '…'
}
