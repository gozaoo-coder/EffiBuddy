<script setup lang="ts">
/**
 * GitQuickActions —— git 版本管理「快速操作」卡片
 *
 * 职责：提供高频操作的聚合入口（仅仓库已初始化时由父组件渲染）：
 * - 保存快照：大按钮 + 可选备注（commit message）
 * - 开分支：输入分支名 → checkout -b 并切换
 * - 撤回：撤销最近一次保存（soft reset，改动保留）
 *
 * 备注/分支名输入框使用 defineModel 与父组件双向绑定，
 * 便于父组件操作成功后统一清空输入。
 */
import { computed } from 'vue'
import { Icon, Button } from '../basic'
import type { GitRepoInfo } from '../../composables/useGitContext'

const props = defineProps<{
  status: GitRepoInfo | null
  saveBusy: boolean
  branchBusy: boolean
  revertBusy: boolean
}>()

const emit = defineEmits<{
  (e: 'save', message: string): void
  (e: 'create-branch', name: string): void
  (e: 'revert-recent'): void
}>()

const commitMsg = defineModel<string>('commitMsg', { default: '' })
const branchName = defineModel<string>('branchName', { default: '' })

const canRevert = computed(
  () => (props.status?.commits.length ?? 0) > 0 && !props.status?.is_effisuite_project,
)
const projectLocked = computed(() => !!props.status?.is_effisuite_project)

function onSave() {
  emit('save', commitMsg.value.trim())
}

function onBranch() {
  emit('create-branch', branchName.value.trim())
}
</script>

<template>
  <div class="git-actions">
    <!-- 保存快照 -->
    <section class="ga-card ga-card--hl">
      <div class="ga-head">
        <span class="ga-title"><Icon name="bookmark" :size="15" />保存快照</span>
        <span class="ga-tag">记录当前版本点</span>
      </div>
      <div class="ga-input-row">
        <input
          v-model="commitMsg"
          type="text"
          class="ga-input"
          placeholder="保存备注（可选）…"
          @keydown.enter="onSave"
        />
        <Button size="sm" variant="primary" :loading="saveBusy" @click="onSave">保存</Button>
      </div>
      <p class="ga-hint">把当前改动 commit 为一个快照点，之后可随时回溯。</p>
    </section>

    <!-- 开分支 -->
    <section class="ga-card">
      <div class="ga-head">
        <span class="ga-title"><Icon name="branch" :size="15" />开分支</span>
      </div>
      <div class="ga-input-row">
        <input
          v-model="branchName"
          type="text"
          class="ga-input"
          placeholder="新分支名…"
          @keydown.enter="onBranch"
        />
        <Button size="sm" :loading="branchBusy" @click="onBranch">开分支</Button>
      </div>
      <p class="ga-hint">从当前版本开一条新分支并切换，便于实验性修改。</p>
    </section>

    <!-- 撤回 -->
    <section class="ga-card ga-card--danger">
      <div class="ga-head">
        <span class="ga-title"><Icon name="undo" :size="15" />撤回</span>
      </div>
      <div class="ga-danger-row">
        <Button
          size="sm"
          variant="danger"
          :disabled="!canRevert"
          :loading="revertBusy"
          @click="emit('revert-recent')"
        >
          撤回最近保存
        </Button>
      </div>
      <p v-if="projectLocked" class="ga-hint ga-hint--danger">
        EffiSuite 项目仓库禁止撤回（后端硬性约束）。
      </p>
      <p v-else class="ga-hint">
        撤销最近一次提交，改动保留在暂存区，可重新保存。
      </p>
    </section>
  </div>
</template>

<style scoped>
.git-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* ---------- 卡片骨架 ---------- */
.ga-card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 10px 12px;
}

.ga-card--hl {
  border-color: color-mix(in srgb, var(--primary) 32%, var(--border));
}

.ga-card--danger {
  border-color: color-mix(in srgb, var(--danger) 22%, var(--border));
}

.ga-head {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
}

.ga-title {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.ga-title :deep(.app-icon) {
  color: var(--primary);
}

.ga-card--danger .ga-title :deep(.app-icon) {
  color: var(--danger);
}

.ga-tag {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.ga-input-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.ga-input {
  flex: 1;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  font-size: var(--fs-sm);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg);
  color: var(--text);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.ga-input::placeholder {
  color: var(--muted);
}

.ga-input:focus {
  border-color: var(--primary);
}

.ga-danger-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.ga-hint {
  margin: 6px 0 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

.ga-hint--danger {
  color: var(--danger);
}
</style>
