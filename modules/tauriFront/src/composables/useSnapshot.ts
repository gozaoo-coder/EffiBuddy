/**
 * useSnapshot —— 会话版本管理（自研内容寻址快照引擎）composable。
 *
 * # 与 useGitContext 的区别
 * - git 版直接在工作区/聊天记录目录跑 git，会污染用户已有仓库；
 * - 本引擎把工作区文件状态做**内容寻址快照**，存到应用私有目录，不写 `.git`、不做
 *   `git add/commit`，对任意目录（含已有 git 仓库的项目）都安全无冲突。
 * - 后端在每次 edit_file / write_file / edit_file_regex / delete_file 等文件写工具
 *   成功后会**自动保存快照**（事件 `session-snapshot-saved`），本 composable 负责
 *   手动保存 / 列表 / 状态 / 恢复 / 删除。
 *
 * # 命令映射
 * - `snapshot_status`   工作区与最新快照差异（dirty / changes）
 * - `snapshot_list`     快照列表（最新在前）
 * - `snapshot_save`     手动保存
 * - `snapshot_restore`  恢复到指定快照（dry_run 预览）
 * - `snapshot_delete`   删除（不可删最新）
 */
import { ref, watch, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

/** 快照来源：auto=agent 工具自动 / manual=手动 / pre_restore=恢复前保护 */
export type SnapshotSource = 'auto' | 'manual' | 'pre_restore'

/** 快照摘要（后端 SnapshotMeta 镜像，created_at 为 Unix 毫秒） */
export interface SnapshotMeta {
  id: string
  created_at: number
  message: string
  source: SnapshotSource
  /** 文件数 */
  files: number
  /** 总字节数（去重后引用体积） */
  bytes: number
}

/** 单个差异项 */
export interface ChangeInfo {
  path: string
  kind: 'added' | 'modified' | 'deleted'
}

/** 工作区快照状态（后端 SnapshotStatus 镜像） */
export interface SnapshotStatus {
  working_dir: string
  dir_exists: boolean
  has_snapshot: boolean
  latest_id: string | null
  /** Unix 毫秒 */
  latest_at: number | null
  dirty: boolean
  changes: ChangeInfo[]
  total: number
}

/** 恢复结果 */
export interface RestoreResult {
  restored: number
  removed: number
  skipped: number
  dry_run: boolean
  message: string
}

export interface UseSnapshot {
  /** 工作区与最新快照的差异状态 */
  status: Ref<SnapshotStatus | null>
  /** 快照列表（最新在前） */
  snapshots: Ref<SnapshotMeta[]>
  loading: Ref<boolean>
  error: Ref<string | null>
  refresh: () => Promise<void>
  /** 手动保存；无改动返回 null */
  save: (message: string) => Promise<SnapshotMeta | null>
  /** 恢复；dryRun=true 只预览。成功返回结果对象 */
  restore: (snapshotId: string, dryRun?: boolean) => Promise<RestoreResult | null>
  /** 删除快照（不可删最新一条） */
  remove: (snapshotId: string) => Promise<boolean>
}

/**
 * 创建会话快照管理实例。
 * @param conversationId 当前会话 id（null / `__` 开头时不加载）
 */
export function useSnapshot(conversationId: Ref<string | null | undefined>): UseSnapshot {
  const status = ref<SnapshotStatus | null>(null)
  const snapshots = ref<SnapshotMeta[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  function isUsable(): boolean {
    const id = conversationId.value
    return !!id && !id.startsWith('__')
  }

  /** 刷新工作区状态 + 快照列表（并行） */
  async function refresh(): Promise<void> {
    if (!isUsable()) {
      status.value = null
      snapshots.value = []
      return
    }
    loading.value = true
    error.value = null
    try {
      const [st, list] = await Promise.all([
        invoke<SnapshotStatus>('snapshot_status', { conversationId: conversationId.value }),
        invoke<SnapshotMeta[]>('snapshot_list', { conversationId: conversationId.value }),
      ])
      status.value = st
      snapshots.value = list
    } catch (e) {
      error.value = String(e)
      status.value = null
      snapshots.value = []
    } finally {
      loading.value = false
    }
  }

  /** 手动保存当前工作区快照 */
  async function save(message: string): Promise<SnapshotMeta | null> {
    if (!isUsable()) return null
    loading.value = true
    error.value = null
    try {
      const meta = await invoke<SnapshotMeta | null>('snapshot_save', {
        conversationId: conversationId.value,
        message,
      })
      await refresh()
      return meta
    } catch (e) {
      error.value = String(e)
      return null
    } finally {
      loading.value = false
    }
  }

  /** 恢复到指定快照 */
  async function restore(snapshotId: string, dryRun = false): Promise<RestoreResult | null> {
    if (!isUsable()) return null
    error.value = null
    try {
      const r = await invoke<RestoreResult>('snapshot_restore', {
        conversationId: conversationId.value,
        snapshotId,
        dryRun,
      })
      // 实际恢复后工作区已变化：刷新状态与列表（恢复前保护快照也会出现在列表）
      if (!dryRun) await refresh()
      return r
    } catch (e) {
      error.value = String(e)
      return null
    }
  }

  /** 删除指定快照 */
  async function remove(snapshotId: string): Promise<boolean> {
    if (!isUsable()) return false
    error.value = null
    try {
      await invoke('snapshot_delete', {
        conversationId: conversationId.value,
        snapshotId,
      })
      await refresh()
      return true
    } catch (e) {
      error.value = String(e)
      return false
    }
  }

  // 会话切换时自动刷新；首次调用立即刷新一次
  watch(conversationId, () => {
    void refresh()
  })
  void refresh()

  return { status, snapshots, loading, error, refresh, save, restore, remove }
}
