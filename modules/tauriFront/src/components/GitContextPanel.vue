<script setup lang="ts">
/**
 * GitContextPanel —— 聊天右栏「版本管理」页签（版本管理编排容器）
 *
 * 两种模式（SegmentedButton 切换）：
 * 1. 会话版本管理（session，默认）：**自研内容寻址快照引擎**。
 *    每次 edit / write 等文件操作后自动保存工作区快照，可随时撤回 / 回溯；
 *    不依赖 git、不写 `.git`，不与工作区已有 git 仓库冲突 → 对任意目录安全。
 *    （UI 细节在 session/ 原子子组件：SessionVersionPanel / SnapshotStatusCard / SnapshotTimeline）
 * 2. 聊天记录目录（chat）：对聊天记录目录做 git 快照 / 分支 / 回溯。
 *    （UI 细节在 git/ 原子子组件：GitStatusCard / GitQuickActions / GitTimeline）
 *
 * 安全边界（后端 git_service 硬性约束，UI 层配合）：
 * - 聊天记录目录模式的撤回 / 回溯为危险操作，均需 Dialog 二次确认
 * - EffiSuite 项目自身仓库禁用撤回 / 回溯（按钮置灰 + 提示，见 GitQuickActions / GitTimeline）
 */
import { ref, computed } from 'vue'
import { Icon, Dialog, SegmentedButton, useToast } from './basic'
import SessionVersionPanel from './session/SessionVersionPanel.vue'
import GitStatusCard from './git/GitStatusCard.vue'
import GitQuickActions from './git/GitQuickActions.vue'
import GitTimeline from './git/GitTimeline.vue'
import { useGitContext, type GitScope } from '../composables/useGitContext'

type VersionMode = 'session' | 'chat'

const props = defineProps<{
  /** 当前会话 id（null 表示未建立会话） */
  conversationId?: string | null
}>()

const { toast } = useToast()

// ===================== 模式 =====================
const mode = ref<VersionMode>('session')
const conversationRef = computed(() => props.conversationId)

// ===================== 聊天记录目录 git 管理（chat 模式专用） =====================
// 范围固定为聊天记录目录；会话版本管理模式用自研快照引擎（useSnapshot），不走 git
const gitScope = computed<GitScope>(() => 'chat')
const {
  status,
  loading,
  error,
  refresh,
  initRepo,
  createBranch,
  save,
  revert,
  checkout,
} = useGitContext(conversationRef, gitScope)

// ===================== 输入 / busy（chat 模式） =====================
const newBranchName = ref('')
const commitMessage = ref('')
const branchBusy = ref(false)
const saveBusy = ref(false)
const revertBusy = ref(false)
/** 时间线单节点 busy（精确到操作类型 + hash） */
const timelineBusy = ref<{ type: 'checkout' | 'revert'; hash: string } | null>(null)

// ===================== 撤回确认对话框（chat 模式） =====================
const revertDialogOpen = ref(false)
/** null = 撤销最近一次保存；string = 恢复到指定提交 */
const revertTarget = ref<string | null>(null)

// ===================== 操作（chat 模式） =====================
async function onInitRepo() {
  await initRepo()
  if (error.value) toast({ content: `初始化仓库失败：${error.value}`, type: 'error' })
  else toast({ content: '仓库已初始化', type: 'success' })
}

async function onCreateBranch(name: string) {
  if (!name) {
    toast({ content: '请输入分支名', type: 'warn' })
    return
  }
  branchBusy.value = true
  const ok = await createBranch(name)
  if (ok) {
    toast({ content: `已切换到新分支「${name}」`, type: 'success' })
    newBranchName.value = ''
  } else if (error.value) {
    toast({ content: `开分支失败：${error.value}`, type: 'error' })
  }
  branchBusy.value = false
}

async function onSave(message: string) {
  saveBusy.value = true
  const r = await save(message)
  if (r) {
    toast({ content: r.message, type: r.committed ? 'success' : 'info' })
    commitMessage.value = ''
  } else if (error.value) {
    toast({ content: `保存失败：${error.value}`, type: 'error' })
  }
  saveBusy.value = false
}

/** 打开撤回确认框：无参 = 撤销最近保存；有参 = 恢复到指定提交 */
function openRevert(commit?: string) {
  revertTarget.value = commit ?? null
  revertDialogOpen.value = true
}

async function confirmRevert() {
  const target = revertTarget.value
  if (target) timelineBusy.value = { type: 'revert', hash: target }
  else revertBusy.value = true
  const ok = await revert(target ?? undefined)
  if (ok) toast({ content: '撤回完成', type: 'success' })
  else if (error.value) toast({ content: `撤回失败：${error.value}`, type: 'error' })
  if (target) timelineBusy.value = null
  else revertBusy.value = false
  revertDialogOpen.value = false
}

async function onCheckout(commit: string) {
  timelineBusy.value = { type: 'checkout', hash: commit }
  const ok = await checkout(commit)
  if (ok) toast({ content: '回溯完成', type: 'success' })
  else if (error.value) toast({ content: `回溯失败：${error.value}`, type: 'error' })
  timelineBusy.value = null
}

/** 撤回确认框文案 */
const revertDialogTitle = computed(() =>
  revertTarget.value ? '确认恢复到该提交' : '确认撤回最近保存',
)
const revertDialogText = computed(() => {
  if (revertTarget.value) {
    return `将把聊天记录目录恢复到提交 ${revertTarget.value} 的文件状态（不改动分支指针与历史）。该操作会覆盖当前文件内容，请确认。`
  }
  return `将撤销最近一次保存，把聊天记录目录回退到上一版本点（改动保留在暂存区，不丢失）。请确认。`
})
</script>

<template>
  <div class="git-panel">
    <!-- 模式切换：会话版本管理（自研快照）/ 聊天记录目录（git） -->
    <div class="gp-mode">
      <SegmentedButton
        v-model="mode"
        :options="[
          { label: '会话版本管理', value: 'session' },
          { label: '聊天记录目录', value: 'chat' },
        ]"
        size="sm"
        block
      />
    </div>

    <!-- ==================== 会话版本管理（自研快照，默认） ==================== -->
    <SessionVersionPanel v-if="mode === 'session'" :conversation-id="conversationId" />

    <!-- ==================== 聊天记录目录（git） ==================== -->
    <template v-else>
      <!-- 状态概览：仓库状态仪表盘 + 未保存横幅 -->
      <GitStatusCard
        :status="status"
        :loading="loading"
        :error="error"
        :save-busy="saveBusy"
        @refresh="refresh"
        @init="onInitRepo"
        @save="onSave('')"
      />

      <!-- 已初始化后：快速操作 -->
      <GitQuickActions
        v-if="status?.is_repo"
        :status="status"
        :save-busy="saveBusy"
        :branch-busy="branchBusy"
        :revert-busy="revertBusy"
        v-model:commit-msg="commitMessage"
        v-model:branch-name="newBranchName"
        @save="onSave"
        @create-branch="onCreateBranch"
        @revert-recent="openRevert()"
      />

      <!-- 已初始化后：提交历史时间线 -->
      <GitTimeline
        v-if="status?.is_repo"
        :status="status"
        :busy="timelineBusy"
        @checkout="onCheckout"
        @revert-to="(h) => openRevert(h)"
      />
    </template>
  </div>

  <!-- 撤回 / 恢复到指定提交 二次确认（聊天记录目录模式） -->
  <Dialog
    v-model:visible="revertDialogOpen"
    :title="revertDialogTitle"
    :danger="true"
    confirm-text="确认撤回"
    :close-on-click-overlay="true"
    @confirm="confirmRevert"
  >
    <div class="git-dialog">
      <div class="git-dialog-icon">
        <Icon name="warning" :size="20" />
      </div>
      <p class="git-dialog-text">{{ revertDialogText }}</p>
    </div>
  </Dialog>
</template>

<style scoped>
.git-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* 模式切换 */
.gp-mode {
  padding: 8px 10px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

/* ---------- 撤回对话框 ---------- */
.git-dialog {
  display: flex;
  gap: 10px;
  align-items: flex-start;
}

.git-dialog-icon {
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

.git-dialog-text {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text);
  line-height: 1.7;
  word-break: break-all;
}
</style>
