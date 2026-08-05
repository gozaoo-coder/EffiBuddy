<script setup lang="ts">
/**
 * SnapshotStatusCard —— 会话版本管理「状态概览」原子卡片
 *
 * 职责（纯展示 + 轻交互，数据/操作由父组件 SessionVersionPanel 下发）：
 * - 工作区路径 + 快照数量 + 最近快照时间
 * - 未保存改动横幅（dirty 时高亮，可展开查看新增/修改/删除明细，一键手动保存）
 * - 手动保存输入框（message + 保存按钮）
 * - 自动保存能力说明（每次 edit / write 等操作自动保存，不与 git 仓库冲突）
 *
 * emits → 父组件：save(message) / refresh
 */
import { ref, computed } from 'vue'
import { Icon, IconButton, Button } from '../basic'
import type { SnapshotStatus, ChangeInfo } from '../../composables/useSnapshot'

const props = defineProps<{
  status: SnapshotStatus | null
  loading: boolean
  error: string | null
  /** 手动保存是否进行中 */
  saveBusy: boolean
}>()

const emit = defineEmits<{
  (e: 'save', message: string): void
  (e: 'refresh'): void
}>()

/** 未保存改动文件列表是否展开 */
const changesExpanded = ref(false)

/** 手动保存备注 */
const saveMessage = ref('')

/** 相对时间（Unix 毫秒 → 相对文案） */
function relTime(ms: number): string {
  if (!ms) return '—'
  const diff = Date.now() - ms
  if (diff < 60_000) return '刚刚'
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`
  if (diff < 86_400_000 * 30) return `${Math.floor(diff / 86_400_000)} 天前`
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

function shortPath(p: string): string {
  if (p.length <= 48) return p
  return '…' + p.slice(-44)
}

/** 差异项 → 标记 / 颜色 / 标签 */
function changeMeta(c: ChangeInfo): { mark: string; cls: string; label: string } {
  if (c.kind === 'added') return { mark: '+', cls: 'added', label: '新增' }
  if (c.kind === 'deleted') return { mark: 'D', cls: 'deleted', label: '删除' }
  return { mark: 'M', cls: 'modified', label: '修改' }
}

function onSave() {
  emit('save', saveMessage.value.trim())
  saveMessage.value = ''
}
</script>

<template>
  <section class="ss-card">
    <div class="ss-head">
      <span class="ss-title"><Icon name="history" :size="15" />会话版本管理</span>
      <div class="ss-actions">
        <IconButton size="sm" icon="refresh" title="刷新" @click="emit('refresh')" />
      </div>
    </div>

    <!-- 加载中 -->
    <div v-if="loading && !status" class="ss-loading">
      <Icon name="loader" :size="16" class="ss-spin" />
      <span>加载中…</span>
    </div>

    <!-- 加载失败 -->
    <div v-else-if="error && !status" class="ss-error">
      <Icon name="warning" :size="15" />
      <span>{{ error }}</span>
    </div>

    <!-- 无会话 / 未设置工作区 -->
    <div v-else-if="!status" class="ss-idle">
      <Icon name="info" :size="15" />
      <span>打开一个会话并设置工作区后，即可使用会话版本管理。</span>
    </div>

    <template v-else>
      <!-- 工作区路径 -->
      <div class="ss-path" :title="status.working_dir">
        <Icon name="folder" :size="13" />
        <span>{{ shortPath(status.working_dir) }}</span>
      </div>

      <!-- 数字统计 -->
      <div class="ss-stats">
        <div class="ss-stat">
          <span class="ss-stat-val">{{ status.total }}</span>
          <span class="ss-stat-label">快照</span>
        </div>
        <div class="ss-stat">
          <span
            class="ss-stat-val"
            :class="{ 'ss-stat-val--warn': status.dirty && status.total > 0 }"
          >
            {{ status.changes.length }}
          </span>
          <span class="ss-stat-label">未保存</span>
        </div>
        <div class="ss-stat">
          <span class="ss-stat-val">{{ relTime(status.latest_at ?? 0) }}</span>
          <span class="ss-stat-label">最近快照</span>
        </div>
      </div>

      <!-- 自动保存说明 -->
      <div class="ss-auto-hint">
        <Icon name="spark" :size="13" class="ss-auto-icon" />
        <span>每次 edit / 写入文件等操作后自动保存工作区快照，可随时撤回；不触碰 git 仓库。</span>
      </div>

      <!-- 无快照 -->
      <div v-if="!status.has_snapshot" class="ss-init">
        <p class="ss-init-text">
          还没有快照。手动保存一次，或让 agent 执行文件操作后自动生成。
        </p>
      </div>

      <!-- 干净 -->
      <div v-else-if="!status.dirty" class="ss-clean">
        <Icon name="check" :size="14" />
        <span>工作区与最近快照一致，无未保存改动</span>
      </div>

      <!-- 未保存改动横幅 -->
      <div v-else class="ss-dirty">
        <div class="ss-dirty-bar" @click="changesExpanded = !changesExpanded">
          <Icon name="warning" :size="15" class="ss-dirty-icon" />
          <span class="ss-dirty-text">{{ status.changes.length }} 处改动未保存</span>
          <IconButton
            size="sm"
            :icon="changesExpanded ? 'chevron-up' : 'chevron-down'"
            :title="changesExpanded ? '收起改动列表' : '展开改动列表'"
            @click.stop="changesExpanded = !changesExpanded"
          />
        </div>
        <Transition name="ss-expand">
          <div v-if="changesExpanded" class="ss-changes">
            <div
              v-for="(c, i) in status.changes"
              :key="i"
              class="ss-change"
              :title="`${changeMeta(c).label}：${c.path}`"
            >
              <span class="ss-change-mark" :class="changeMeta(c).cls">
                {{ changeMeta(c).mark }}
              </span>
              <span class="ss-change-file">{{ c.path }}</span>
            </div>
          </div>
        </Transition>
      </div>

      <!-- 手动保存 -->
      <div class="ss-save">
        <input
          v-model="saveMessage"
          class="ss-save-input"
          placeholder="保存备注（可选）"
          @keydown.enter="onSave"
        />
        <Button size="sm" variant="primary" :loading="saveBusy" @click="onSave">
          <template #icon><Icon name="plus" :size="14" /></template>
          保存
        </Button>
      </div>
    </template>
  </section>
</template>

<style scoped>
.ss-card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.ss-head {
  display: flex;
  align-items: center;
  gap: 6px;
}

.ss-title {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.ss-title :deep(.app-icon) {
  color: var(--primary);
}

.ss-actions {
  margin-left: auto;
  display: flex;
  gap: 2px;
}

/* 加载 / 错误 / 空态 */
.ss-loading,
.ss-error,
.ss-idle {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  color: var(--muted);
  padding: 4px 0;
  line-height: 1.5;
}

.ss-error {
  color: var(--danger);
  align-items: flex-start;
}

.ss-error :deep(.app-icon) {
  flex-shrink: 0;
  margin-top: 1px;
}

.ss-spin {
  animation: ss-rotate 1s linear infinite;
}

@keyframes ss-rotate {
  to {
    transform: rotate(360deg);
  }
}

/* 工作区路径 */
.ss-path {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--muted);
  font-family: var(--font-mono, ui-monospace, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  direction: rtl;
  text-align: left;
}

.ss-path :deep(.app-icon) {
  flex-shrink: 0;
}

/* 数字统计 */
.ss-stats {
  display: grid;
  grid-template-columns: 1fr 1fr 1.4fr;
  gap: 6px;
}

.ss-stat {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 8px;
  background: var(--bg);
  border-radius: var(--radius-md);
  min-width: 0;
}

.ss-stat-val {
  font-size: var(--fs-base);
  font-weight: 700;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ss-stat-val--warn {
  color: var(--warn);
}

.ss-stat-label {
  font-size: var(--fs-xs);
  color: var(--muted);
  white-space: nowrap;
}

/* 自动保存说明 */
.ss-auto-hint {
  display: flex;
  align-items: flex-start;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
  background: color-mix(in srgb, var(--primary) 6%, var(--bg));
  border: 1px solid color-mix(in srgb, var(--primary) 16%, var(--border));
  border-radius: var(--radius-md);
  padding: 5px 8px;
}

.ss-auto-icon {
  color: var(--primary);
  margin-top: 1px;
  flex-shrink: 0;
}

/* 无快照 */
.ss-init {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.ss-init-text {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.6;
}

/* 干净 */
.ss-clean {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--success);
  background: color-mix(in srgb, var(--success) 10%, var(--card));
  border: 1px solid color-mix(in srgb, var(--success) 24%, var(--border));
  border-radius: var(--radius-md);
  padding: 5px 9px;
}

/* 未保存改动横幅 */
.ss-dirty {
  border: 1px solid color-mix(in srgb, var(--warn) 36%, var(--border));
  border-radius: var(--radius-md);
  overflow: hidden;
  background: color-mix(in srgb, var(--warn) 7%, var(--card));
}

.ss-dirty-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  cursor: pointer;
  user-select: none;
}

.ss-dirty-bar:hover {
  background: color-mix(in srgb, var(--warn) 10%, var(--card));
}

.ss-dirty-icon {
  color: var(--warn);
  flex-shrink: 0;
}

.ss-dirty-text {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--warn);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ss-changes {
  border-top: 1px dashed color-mix(in srgb, var(--warn) 30%, var(--border));
  max-height: 160px;
  overflow-y: auto;
  padding: 4px 8px 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.ss-change {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.ss-change-mark {
  font-size: var(--fs-xs);
  font-weight: 700;
  width: 16px;
  height: 16px;
  border-radius: 3px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  font-family: var(--font-mono, ui-monospace, monospace);
}

.ss-change-mark.added {
  background: color-mix(in srgb, var(--success) 16%, var(--card));
  color: var(--success);
}

.ss-change-mark.deleted {
  background: color-mix(in srgb, var(--danger) 16%, var(--card));
  color: var(--danger);
}

.ss-change-mark.modified {
  background: color-mix(in srgb, var(--warn) 16%, var(--card));
  color: var(--warn);
}

.ss-change-file {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-xs);
  color: var(--text);
  font-family: var(--font-mono, ui-monospace, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 手动保存 */
.ss-save {
  display: flex;
  gap: 6px;
}

.ss-save-input {
  flex: 1;
  min-width: 0;
  padding: 5px 9px;
  font-size: var(--fs-xs);
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color 0.15s ease;
}

.ss-save-input:focus {
  border-color: var(--primary);
}

/* 展开过渡 */
.ss-expand-enter-active,
.ss-expand-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}

.ss-expand-enter-from,
.ss-expand-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
