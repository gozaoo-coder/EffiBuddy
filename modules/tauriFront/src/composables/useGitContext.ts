/**
 * useGitContext —— 基于 git 的上下文版本管理 composable。
 *
 * # 职责
 * - 封装后端 git_context_* 系列 Tauri 命令（status/init/branch/save/revert/checkout/history）
 * - 持有响应式仓库状态（当前分支 / 未提交改动 / 历史提交列表）
 * - 会话（conversationId）或范围（chat / workspace）变化时自动刷新
 *
 * # 范围说明
 * - `chat`：聊天记录目录（`<appdata>/effisuite/conversations`）
 * - `workspace`：当前会话的工作区目录（未设置工作区时后端会报错）
 *
 * # 设计
 * - 实例级状态：每个使用处（GitContextPanel）一份，互不污染
 * - 所有命令成功后内部刷新 `status`，UI 只需读取响应式状态
 * - 方法返回 `boolean` / 结果对象表示是否成功，错误存于 `error` ref 供 toast
 */
import { ref, watch, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export type GitScope = 'chat' | 'workspace'

/** 单个历史提交 */
export interface GitCommitInfo {
  hash: string
  message: string
  /** Unix 秒 */
  timestamp: number
}

/** 仓库状态快照（后端 GitRepoInfo 的镜像） */
export interface GitRepoInfo {
  path: string
  repo_root: string
  is_repo: boolean
  head_hash: string
  branch: string | null
  detached: boolean
  is_effisuite_project: boolean
  dirty: boolean
  changed: string[]
  commits: GitCommitInfo[]
}

/** 保存（commit）结果 */
export interface GitSaveResult {
  committed: boolean
  hash: string | null
  message: string
}

export interface UseGitContext {
  /** 仓库状态（未初始化或失败时为 null） */
  status: Ref<GitRepoInfo | null>
  loading: Ref<boolean>
  /** 最近一次操作的错误信息（成功时为 null） */
  error: Ref<string | null>
  refresh: () => Promise<void>
  initRepo: () => Promise<void>
  createBranch: (name: string) => Promise<boolean>
  save: (message: string) => Promise<GitSaveResult | null>
  revert: (commit?: string) => Promise<boolean>
  checkout: (commit: string) => Promise<boolean>
}

/**
 * 创建 git 上下文管理实例。
 *
 * @param conversationId 当前会话 id（null / `__` 开头时不加载）
 * @param scope 仓库范围：chat 聊天记录 / workspace 工作区
 */
export function useGitContext(
  conversationId: Ref<string | null | undefined>,
  scope: Ref<GitScope>,
): UseGitContext {
  const status = ref<GitRepoInfo | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  function isUsable(): boolean {
    const id = conversationId.value
    return !!id && !id.startsWith('__')
  }

  /** 刷新仓库状态 */
  async function refresh(): Promise<void> {
    if (!isUsable()) {
      status.value = null
      return
    }
    loading.value = true
    error.value = null
    try {
      status.value = await invoke<GitRepoInfo>('git_context_status', {
        scope: scope.value,
        conversationId: conversationId.value,
      })
    } catch (e) {
      error.value = String(e)
      status.value = null
    } finally {
      loading.value = false
    }
  }

  /** 初始化仓库（git init + 首次提交）；成功后自动刷新状态 */
  async function initRepo(): Promise<void> {
    if (!isUsable()) return
    loading.value = true
    error.value = null
    try {
      status.value = await invoke<GitRepoInfo>('git_context_init', {
        scope: scope.value,
        conversationId: conversationId.value,
      })
    } catch (e) {
      error.value = String(e)
    } finally {
      loading.value = false
    }
  }

  /** 开分支并切换 */
  async function createBranch(name: string): Promise<boolean> {
    if (!isUsable()) return false
    loading.value = true
    error.value = null
    try {
      status.value = await invoke<GitRepoInfo>('git_context_branch', {
        scope: scope.value,
        conversationId: conversationId.value,
        name,
      })
      return true
    } catch (e) {
      error.value = String(e)
      return false
    } finally {
      loading.value = false
    }
  }

  /** 保存快照（commit）；无改动时返回 committed=false */
  async function save(message: string): Promise<GitSaveResult | null> {
    if (!isUsable()) return null
    loading.value = true
    error.value = null
    try {
      const r = await invoke<GitSaveResult>('git_context_save', {
        scope: scope.value,
        conversationId: conversationId.value,
        message,
      })
      status.value = await invoke<GitRepoInfo>('git_context_status', {
        scope: scope.value,
        conversationId: conversationId.value,
      })
      return r
    } catch (e) {
      error.value = String(e)
      return null
    } finally {
      loading.value = false
    }
  }

  /** 撤回：不传 commit 撤销最近一次保存；传 commit 恢复到该提交的文件状态 */
  async function revert(commit?: string): Promise<boolean> {
    if (!isUsable()) return false
    loading.value = true
    error.value = null
    try {
      status.value = await invoke<GitRepoInfo>('git_context_revert', {
        scope: scope.value,
        conversationId: conversationId.value,
        commit: commit ?? null,
      })
      return true
    } catch (e) {
      error.value = String(e)
      return false
    } finally {
      loading.value = false
    }
  }

  /** 回溯到指定提交（detached HEAD） */
  async function checkout(commit: string): Promise<boolean> {
    if (!isUsable()) return false
    loading.value = true
    error.value = null
    try {
      status.value = await invoke<GitRepoInfo>('git_context_checkout', {
        scope: scope.value,
        conversationId: conversationId.value,
        commit,
      })
      return true
    } catch (e) {
      error.value = String(e)
      return false
    } finally {
      loading.value = false
    }
  }

  // 会话或范围变化时自动刷新；首次调用立即刷新一次
  watch([conversationId, scope], () => {
    void refresh()
  })
  void refresh()

  return { status, loading, error, refresh, initRepo, createBranch, save, revert, checkout }
}
