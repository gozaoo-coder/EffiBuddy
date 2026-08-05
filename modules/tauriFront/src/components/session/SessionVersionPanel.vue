<script setup lang="ts">
/**
 * SessionVersionPanel —— 会话版本管理主面板（编排层）
 *
 * 职责（编排，不承载 UI 细节）：
 * - 持有 useSnapshot 状态与 save / restore / delete busy 状态，编排操作流程
 * - 组合原子子组件（单一原子文件原则，UI 细节下沉）：
 *   · SnapshotStatusCard —— 工作区状态 + 未保存改动 + 手动保存
 *   · SnapshotTimeline   —— 快照时间线（恢复 / 删除）
 * - 恢复（回溯）为危险操作：统一在此先 dry-run 预览 → Dialog 二次确认 → 实际恢复
 * - 监听后端 `session-snapshot-saved` 事件（agent 每次 edit/write 后自动保存）实时刷新
 *
 * 安全边界：
 * - 恢复前后端自动保存「保护快照」，任何恢复都可再撤回
 * - 最新一条快照不可删除（后端硬性约束，UI 同步置灰）
 */
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Icon, Dialog, useToast } from '../basic'
import SnapshotStatusCard from './SnapshotStatusCard.vue'
import SnapshotTimeline from './SnapshotTimeline.vue'
import { useSnapshot } from '../../composables/useSnapshot'

const props = defineProps<{
  /** 当前会话 id（null 表示未建立会话） */
  conversationId?: string | null
}>()

const { toast } = useToast()

const conversationRef = computed(() => props.conversationId)

// ===================== composable =====================
const { status, snapshots, loading, error, refresh, save, restore, remove } =
  useSnapshot(conversationRef)

// ===================== busy / 输入 =====================
const saveBusy = ref(false)
/** 时间线单节点 busy（精确到操作类型 + 快照 id） */
const timelineBusy = ref<{ type: 'restore' | 'delete'; id: string } | null>(null)

// ===================== 恢复确认对话框 =====================
const restoreDialogOpen = ref(false)
/** 待恢复的快照 */
const restoreTarget = ref<{ id: string; message: string } | null>(null)
/** dry-run 预览结果（确认框内展示将发生的操作） */
const restorePreview = ref<string | null>(null)

// ===================== 实时刷新：agent 自动保存后同步 =====================
let unlisten: UnlistenFn | null = null

onMounted(() => {
  listen<unknown>('session-snapshot-saved', () => {
    void refresh()
  }).then((fn) => {
    unlisten = fn
  })
})

onUnmounted(() => {
  unlisten?.()
})

// ===================== 操作 =====================
/** 手动保存 */
async function onSave(message: string) {
  saveBusy.value = true
  const meta = await save(message)
  if (meta) {
    toast({
      content: meta.message ? `已保存快照：${meta.message}` : '已保存快照',
      type: 'success',
    })
  } else if (error.value) {
    toast({ content: `保存失败：${error.value}`, type: 'error' })
  } else {
    toast({ content: '工作区无改动，未生成新快照', type: 'info' })
  }
  saveBusy.value = false
}

/** 打开恢复确认框：先 dry-run 预览 */
async function openRestore(id: string) {
  const target = snapshots.value.find((s) => s.id === id)
  if (!target) return
  restoreTarget.value = { id, message: target.message }
  restorePreview.value = null
  restoreDialogOpen.value = true
  timelineBusy.value = { type: 'restore', id }
  const r = await restore(id, true)
  if (r) {
    restorePreview.value = r.message
  } else if (error.value) {
    restorePreview.value = `预览失败：${error.value}`
  }
  timelineBusy.value = null
}

/** 确认恢复 */
async function confirmRestore() {
  const target = restoreTarget.value
  if (!target) return
  restoreDialogOpen.value = false
  timelineBusy.value = { type: 'restore', id: target.id }
  const r = await restore(target.id, false)
  if (r) {
    toast({ content: r.message, type: 'success' })
  } else if (error.value) {
    toast({ content: `恢复失败：${error.value}`, type: 'error' })
  }
  timelineBusy.value = null
  restoreTarget.value = null
  restorePreview.value = null
}

/** 删除快照 */
async function onDelete(id: string) {
  timelineBusy.value = { type: 'delete', id }
  const ok = await remove(id)
  if (ok) toast({ content: '快照已删除', type: 'success' })
  else if (error.value) toast({ content: `删除失败：${error.value}`, type: 'error' })
  timelineBusy.value = null
}

const restoreDialogTitle = computed(() =>
  restoreTarget.value ? '确认恢复到该快照' : '恢复快照',
)
</script>

<template>
  <div class="sv-panel">
    <!-- 状态概览 + 手动保存 -->
    <SnapshotStatusCard
      :status="status"
      :loading="loading"
      :error="error"
      :save-busy="saveBusy"
      @save="onSave"
      @refresh="refresh"
    />

    <!-- 快照时间线 -->
    <SnapshotTimeline
      :snapshots="snapshots"
      :loading="loading"
      :busy="timelineBusy"
      @restore="openRestore"
      @delete="onDelete"
    />
  </div>

  <!-- 恢复确认 -->
  <Dialog
    v-model:visible="restoreDialogOpen"
    :title="restoreDialogTitle"
    :danger="true"
    confirm-text="确认恢复"
    :close-on-click-overlay="true"
    @confirm="confirmRestore"
  >
    <div class="sv-dialog">
      <div class="sv-dialog-icon">
        <Icon name="warning" :size="20" />
      </div>
      <div class="sv-dialog-body">
        <p class="sv-dialog-text">
          {{
            restoreTarget
              ? `将把工作区恢复到快照「${restoreTarget.message}」的文件状态。`
              : ''
          }}
          恢复前会自动保存一次「保护快照」，因此本次恢复也可以再撤回。
        </p>
        <div v-if="restorePreview" class="sv-dialog-preview">{{ restorePreview }}</div>
      </div>
    </div>
  </Dialog>
</template>

<style scoped>
.sv-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* ---------- 恢复对话框 ---------- */
.sv-dialog {
  display: flex;
  gap: 10px;
  align-items: flex-start;
}

.sv-dialog-icon {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--danger) 14%, var(--card));
  color: var(--danger);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.sv-dialog-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.sv-dialog-text {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text);
  line-height: 1.7;
  word-break: break-all;
}

.sv-dialog-preview {
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 6px 9px;
  line-height: 1.6;
}
</style>
