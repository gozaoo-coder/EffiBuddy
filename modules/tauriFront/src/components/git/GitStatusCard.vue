<script setup lang="ts">
/**
 * GitStatusCard —— 聊天记录目录「仓库状态概览」原子卡片
 *
 * 职责（纯展示 + 轻交互，数据/操作由父组件 GitContextPanel 下发）：
 * - 仓库状态仪表盘：当前分支 + 状态徽章 + 数字统计（提交数 / 未保存数 / 最近保存）+ 仓库路径
 * - 未保存改动横幅：dirty 时高亮提示、可展开查看改动文件明细、一键保存
 *
 * 管理对象固定为「聊天记录目录」（模式切换在 GitContextPanel 容器层）
 *
 * emits → 父组件：refresh / init / save
 */
import { ref, computed } from 'vue'
import { Icon, IconButton, Button } from '../basic'
import type { GitRepoInfo } from '../../composables/useGitContext'

const props = defineProps<{
  status: GitRepoInfo | null
  loading: boolean
  error: string | null
  saveBusy: boolean
}>()

const emit = defineEmits<{
  (e: 'refresh'): void
  (e: 'init'): void
  (e: 'save'): void
}>()

/** 未保存改动文件列表是否展开 */
const changesExpanded = ref(false)

const isProjectRepo = computed(() => !!props.status?.is_effisuite_project)

/** 管理对象中文名（提示文案用） */
const scopeName = '聊天记录目录'

function shortPath(p: string): string {
  if (p.length <= 52) return p
  return '…' + p.slice(-48)
}

/** 相对时间：x 分钟前 / x 小时前 / x 天前 / 日期 */
function relTime(ts: number): string {
  if (!ts) return '—'
  const diff = Date.now() / 1000 - ts
  if (diff < 60) return '刚刚'
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)} 天前`
  const d = new Date(ts * 1000)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

/** porcelain 状态码 → 语义（标记 / 颜色 / 标签） */
interface ChangeMeta {
  mark: string
  cls: 'added' | 'deleted' | 'renamed' | 'modified'
  label: string
}

function changeMeta(line: string): ChangeMeta {
  const codes = line.slice(0, 2)
  if (codes.includes('?')) return { mark: '+', cls: 'added', label: '新增' }
  if (codes.includes('D')) return { mark: 'D', cls: 'deleted', label: '删除' }
  if (codes.includes('R')) return { mark: 'R', cls: 'renamed', label: '重命名' }
  if (codes.includes('M') || codes.includes('A')) return { mark: 'M', cls: 'modified', label: '修改' }
  return { mark: 'M', cls: 'modified', label: '修改' }
}

/** 提取 porcelain 行中的文件路径（去掉前两位状态码；重命名取目标路径） */
function filePath(line: string): string {
  const body = line.slice(2).trim()
  return body.split(' -> ').pop() ?? body
}
</script>

<template>
  <div class="git-status">
    <!-- 仓库状态仪表盘（管理对象：聊天记录目录，见模式切换） -->

    <!-- 仓库状态仪表盘 -->
    <section class="gs-card">
      <div class="gs-head">
        <span class="gs-title"><Icon name="branch" :size="15" />仓库状态</span>
        <div class="gs-actions">
          <IconButton size="sm" icon="refresh" title="刷新" @click="emit('refresh')" />
        </div>
      </div>

      <!-- 加载中 -->
      <div v-if="loading && !status" class="gs-loading">
        <Icon name="loader" :size="16" class="gs-spin" />
        <span>加载中…</span>
      </div>

      <!-- 加载失败 -->
      <div v-else-if="error && !status" class="gs-error">
        <Icon name="warning" :size="15" />
        <span>{{ error }}</span>
      </div>

        <template v-else-if="status">
          <!-- 未初始化 -->
          <div v-if="!status.is_repo" class="gs-init">
            <p class="gs-init-text">
              尚未初始化仓库。初始化后可对{{ scopeName }}做版本快照、分支与回溯。
            </p>
            <Button size="sm" variant="primary" block @click="emit('init')">
              <template #icon><Icon name="plus" :size="15" /></template>
              初始化仓库
            </Button>
          </div>

          <!-- 已初始化：仪表盘 -->
          <template v-else>
            <div class="gs-branch-row">
              <Icon name="branch" :size="15" class="gs-branch-icon" />
              <span class="gs-branch-name">{{ status.branch ?? '(detached)' }}</span>
              <span v-if="status.detached" class="gs-badge gs-badge--warn">回溯中</span>
              <span v-if="isProjectRepo" class="gs-badge gs-badge--warn">项目仓库</span>
            </div>

            <!-- 数字统计 -->
            <div class="gs-stats">
              <div class="gs-stat">
                <span class="gs-stat-val">{{ status.commits.length }}</span>
                <span class="gs-stat-label">提交</span>
              </div>
              <div class="gs-stat">
                <span
                  class="gs-stat-val"
                  :class="{ 'gs-stat-val--warn': status.changed.length > 0 }"
                >
                  {{ status.changed.length }}
                </span>
                <span class="gs-stat-label">未保存</span>
              </div>
              <div class="gs-stat">
                <span class="gs-stat-val">{{ relTime(status.commits[0]?.timestamp ?? 0) }}</span>
                <span class="gs-stat-label">最近保存</span>
              </div>
            </div>

            <div class="gs-path" :title="status.path">{{ shortPath(status.path) }}</div>

            <!-- 干净 -->
            <div v-if="!status.dirty" class="gs-clean">
              <Icon name="check" :size="14" />
              <span>工作区干净，无未保存改动</span>
            </div>

            <!-- 未保存改动横幅 -->
            <div v-else class="gs-dirty">
              <div class="gs-dirty-bar" @click="changesExpanded = !changesExpanded">
                <Icon name="warning" :size="15" class="gs-dirty-icon" />
                <span class="gs-dirty-text">{{ status.changed.length }} 处改动未保存</span>
                <span class="gs-dirty-ops" @click.stop>
                  <Button size="sm" variant="primary" :loading="saveBusy" @click="emit('save')">
                    保存
                  </Button>
                  <IconButton
                    size="sm"
                    :icon="changesExpanded ? 'chevron-up' : 'chevron-down'"
                    :title="changesExpanded ? '收起改动列表' : '展开改动列表'"
                    @click="changesExpanded = !changesExpanded"
                  />
                </span>
              </div>
              <Transition name="gs-expand">
                <div v-if="changesExpanded" class="gs-changes">
                  <div
                    v-for="(line, i) in status.changed"
                    :key="i"
                    class="gs-change"
                    :title="`${changeMeta(line).label}：${filePath(line)}`"
                  >
                    <span class="gs-change-mark" :class="changeMeta(line).cls">
                      {{ changeMeta(line).mark }}
                    </span>
                    <span class="gs-change-file">{{ filePath(line) }}</span>
                  </div>
                </div>
              </Transition>
            </div>
          </template>
        </template>

        <!-- 无会话 / 无数据 -->
        <div v-else class="gs-idle">
          <Icon name="info" :size="15" />
          <span>打开一个会话后即可使用版本管理。</span>
        </div>
    </section>
  </div>
</template>

<style scoped>
.git-status {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* ---------- 卡片骨架 ---------- */
.gs-card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 10px 12px;
}

.gs-head {
  display: flex;
  align-items: center;
  gap: 6px;
    margin-bottom: 8px;
}

.gs-title {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.gs-title :deep(.app-icon) {
  color: var(--primary);
}

.gs-actions {
  margin-left: auto;
  display: flex;
  gap: 2px;
}

.gs-hint {
    margin: 6px 0 0;
    font-size: var(--fs-xs);
    color: var(--muted);
    line-height: 1.5;
  }

/* ---------- 加载 / 错误 ---------- */
.gs-loading,
.gs-error {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  color: var(--muted);
  padding: 8px 0;
  line-height: 1.5;
}

.gs-error {
  color: var(--danger);
  align-items: flex-start;
}

.gs-error :deep(.app-icon) {
  flex-shrink: 0;
  margin-top: 1px;
}

.gs-spin {
  animation: gs-rotate 1s linear infinite;
}

@keyframes gs-rotate {
  to {
    transform: rotate(360deg);
  }
}

/* ---------- 无会话 / 无数据 ---------- */
.gs-idle {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  color: var(--muted);
  padding: 8px 0;
  line-height: 1.5;
}

.gs-idle :deep(.app-icon) {
  flex-shrink: 0;
}
/* ---------- 未初始化 ---------- */
.gs-init {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.gs-init-text {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text);
  line-height: 1.6;
}

/* ---------- 仪表盘 ---------- */
.gs-branch-row {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  margin-bottom: 8px;
}

.gs-branch-icon {
  color: var(--primary);
  flex-shrink: 0;
}

.gs-branch-name {
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text);
  font-family: var(--font-mono, ui-monospace, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.gs-badge {
  font-size: var(--fs-xs);
  padding: 1px 8px;
  border-radius: var(--radius-full);
  flex-shrink: 0;
}

.gs-badge--warn {
  background: color-mix(in srgb, var(--warn) 16%, var(--card));
  color: var(--warn);
}

/* 数字统计 */
.gs-stats {
  display: grid;
  grid-template-columns: 1fr 1fr 1.4fr;
  gap: 6px;
  margin-bottom: 8px;
}

.gs-stat {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 8px;
  background: var(--bg);
  border-radius: var(--radius-sm);
  min-width: 0;
}

.gs-stat-val {
  font-size: var(--fs-base);
  font-weight: 700;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.gs-stat-val--warn {
  color: var(--warn);
}

.gs-stat-label {
  font-size: var(--fs-xs);
  color: var(--muted);
  white-space: nowrap;
}

.gs-path {
  margin-bottom: 8px;
  font-size: var(--fs-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  direction: rtl;
  text-align: left;
  font-family: var(--font-mono, ui-monospace, monospace);
}

/* 干净状态 */
.gs-clean {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--success);
  background: color-mix(in srgb, var(--success) 10%, var(--card));
  border: 1px solid color-mix(in srgb, var(--success) 24%, var(--border));
  border-radius: var(--radius-sm);
  padding: 5px 9px;
}

/* 未保存改动横幅 */
.gs-dirty {
  border: 1px solid color-mix(in srgb, var(--warn) 36%, var(--border));
  border-radius: var(--radius-sm);
  overflow: hidden;
  background: color-mix(in srgb, var(--warn) 7%, var(--card));
}

.gs-dirty-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  cursor: pointer;
  user-select: none;
}

.gs-dirty-bar:hover {
  background: color-mix(in srgb, var(--warn) 10%, var(--card));
}

.gs-dirty-icon {
  color: var(--warn);
  flex-shrink: 0;
}

.gs-dirty-text {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--warn);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.gs-dirty-ops {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

/* 展开的改动文件列表 */
.gs-changes {
  border-top: 1px dashed color-mix(in srgb, var(--warn) 30%, var(--border));
  max-height: 150px;
  overflow-y: auto;
  padding: 4px 8px 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.gs-change {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.gs-change-mark {
  font-size: var(--fs-xs);
  font-weight: 700;
  width: 16px;
  height: 16px;
  border-radius: 4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  font-family: var(--font-mono, ui-monospace, monospace);
}

.gs-change-mark.added {
  background: color-mix(in srgb, var(--success) 16%, var(--card));
  color: var(--success);
}

.gs-change-mark.deleted {
  background: color-mix(in srgb, var(--danger) 16%, var(--card));
  color: var(--danger);
}

.gs-change-mark.renamed {
  background: color-mix(in srgb, var(--info) 16%, var(--card));
  color: var(--info);
}

.gs-change-mark.modified {
  background: color-mix(in srgb, var(--warn) 16%, var(--card));
  color: var(--warn);
}

.gs-change-file {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-xs);
  color: var(--text);
  font-family: var(--font-mono, ui-monospace, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 展开过渡 */
.gs-expand-enter-active,
.gs-expand-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}

.gs-expand-enter-from,
.gs-expand-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
