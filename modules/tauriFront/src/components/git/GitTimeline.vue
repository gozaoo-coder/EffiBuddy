<script setup lang="ts">
/**
 * GitTimeline —— git 版本管理「提交历史」可视化时间线
 *
 * 可视化设计：
 * - 垂直时间线：左侧为连续竖线 + 圆点节点，右侧为提交内容
 * - 时间分组：今天 / 昨天 / 本周 / 更早，组标题悬浮于时间线
 * - HEAD 节点：primary 高亮圆点 + 发光光晕 + 「HEAD」标签，一眼定位当前版本
 * - hover 交互：节点浮现「回溯」「撤回到此」操作，操作 loading 精确到单节点
 *
 * emits → 父组件：checkout（回溯）/ revert-to（恢复到指定提交）
 */
import { computed } from 'vue'
import { Icon, Button } from '../basic'
import type { GitRepoInfo, GitCommitInfo } from '../../composables/useGitContext'

const props = defineProps<{
  status: GitRepoInfo | null
  /** 当前正在执行的节点操作（loading 精确到单节点） */
  busy: { type: 'checkout' | 'revert'; hash: string } | null
}>()

const emit = defineEmits<{
  (e: 'checkout', hash: string): void
  (e: 'revert-to', hash: string): void
}>()

const commits = computed(() => props.status?.commits ?? [])
const projectLocked = computed(() => !!props.status?.is_effisuite_project)
const headHash = computed(() => props.status?.head_hash ?? '')
const detached = computed(() => !!props.status?.detached)

type GroupKey = '今天' | '昨天' | '本周' | '更早'

function groupKey(ts: number): GroupKey {
  const now = new Date()
  const startToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000
  if (ts >= startToday) return '今天'
  if (ts >= startToday - 86400) return '昨天'
  if (ts >= startToday - 7 * 86400) return '本周'
  return '更早'
}

interface Grouped {
  key: GroupKey
  items: GitCommitInfo[]
}

/** 按固定分组顺序组装（保持提交原有顺序） */
const groups = computed<Grouped[]>(() => {
  const order: GroupKey[] = ['今天', '昨天', '本周', '更早']
  const map = new Map<GroupKey, GitCommitInfo[]>()
  for (const c of commits.value) {
    const k = groupKey(c.timestamp)
    const arr = map.get(k)
    if (arr) arr.push(c)
    else map.set(k, [c])
  }
  return order
    .filter((k) => map.has(k))
    .map((k) => ({ key: k, items: map.get(k)! }))
})

function isHead(c: GitCommitInfo): boolean {
  return c.hash === headHash.value
}

function isBusy(c: GitCommitInfo, type: 'checkout' | 'revert'): boolean {
  return props.busy?.type === type && props.busy.hash === c.hash
}

/** 相对时间：x 分钟前 / x 小时前 / x 天前 / 月日 */
function relTime(ts: number): string {
  const diff = Date.now() / 1000 - ts
  if (diff < 60) return '刚刚'
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)} 天前`
  const d = new Date(ts * 1000)
  return `${d.getMonth() + 1}月${d.getDate()}日`
}

function absTime(ts: number): string {
  const d = new Date(ts * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

function shortHash(h: string): string {
  return h.length > 7 ? h.slice(0, 7) : h
}
</script>

<template>
  <div class="git-timeline">
    <!-- 头部 -->
    <div class="gt-head">
      <span class="gt-title"><Icon name="history" :size="15" />提交历史</span>
      <span class="gt-count">{{ commits.length }} 条</span>
    </div>

    <!-- 空状态 -->
    <div v-if="commits.length === 0" class="gt-empty">
      <Icon name="git-commit" :size="24" class="gt-empty-icon" />
      <p class="gt-empty-title">暂无提交记录</p>
      <p class="gt-empty-sub">点击「保存快照」创建第一个版本点。</p>
    </div>

    <!-- 时间线 -->
    <div v-else class="gt-groups">
      <div v-for="g in groups" :key="g.key" class="gt-group">
        <div class="gt-group-label">{{ g.key }}</div>
        <div class="gt-list">
          <div
            v-for="(c, i) in g.items"
            :key="c.hash"
            class="gt-node"
            :class="{ 'gt-node--head': isHead(c) }"
          >
            <div class="gt-rail">
              <span class="gt-dot" :class="{ 'gt-dot--head': isHead(c) }"></span>
              <span v-if="i < g.items.length - 1" class="gt-rail-line"></span>
            </div>

            <div class="gt-body">
              <div class="gt-body-row">
                <span v-if="isHead(c)" class="gt-head-tag">
                  <Icon name="branch" :size="12" />
                  HEAD{{ detached ? '（回溯中）' : '' }}
                </span>
                <span class="gt-msg" :title="c.message">{{ c.message || '(无备注)' }}</span>
              </div>
              <div class="gt-meta">
                <span class="gt-hash" :title="c.hash">{{ shortHash(c.hash) }}</span>
                <span class="gt-time" :title="absTime(c.timestamp)">{{ relTime(c.timestamp) }}</span>
              </div>
              <div class="gt-ops">
                <Button
                  size="sm"
                  variant="text"
                  :disabled="projectLocked"
                  :loading="isBusy(c, 'checkout')"
                  @click="emit('checkout', c.hash)"
                >
                  回溯
                </Button>
                <Button
                  size="sm"
                  variant="text"
                  :disabled="projectLocked"
                  :loading="isBusy(c, 'revert')"
                  @click="emit('revert-to', c.hash)"
                >
                  撤回到此
                </Button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.git-timeline {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* ---------- 头部 ---------- */
.gt-head {
  display: flex;
  align-items: center;
  gap: 6px;
}

.gt-title {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.gt-title :deep(.app-icon) {
  color: var(--primary);
}

.gt-count {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 1px 8px;
}

/* ---------- 空状态 ---------- */
.gt-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 18px 12px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  text-align: center;
}

.gt-empty-icon {
  color: var(--muted);
  margin-bottom: 6px;
}

.gt-empty-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.gt-empty-sub {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* ---------- 分组 ---------- */
.gt-groups {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.gt-group-label {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--muted);
  padding: 2px 0 0;
  user-select: none;
}

.gt-list {
  display: flex;
  flex-direction: column;
}

/* ---------- 节点 ---------- */
.gt-node {
  display: flex;
  gap: 10px;
  position: relative;
  border-radius: var(--radius-sm);
  transition: background var(--duration-fast) var(--ease-standard);
}

.gt-node:hover {
  background: var(--bg-2);
}

/* 左轨：圆点 + 竖线 */
.gt-rail {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 14px;
  flex-shrink: 0;
  padding-top: 6px;
}

.gt-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--border);
  border: 1px solid var(--muted);
  flex-shrink: 0;
  z-index: 1;
}

.gt-node--head .gt-dot {
  background: var(--primary);
  border-color: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 22%, transparent),
    0 0 10px color-mix(in srgb, var(--primary) 45%, transparent);
}

.gt-rail-line {
  width: 2px;
  flex: 1;
  min-height: 8px;
  background: var(--border);
  margin-top: 2px;
}

/* 右体 */
.gt-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 6px 8px 8px 2px;
}

.gt-body-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.gt-head-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: 10px;
  font-weight: 700;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  border-radius: var(--radius-full);
  padding: 1px 7px;
  flex-shrink: 0;
}

.gt-msg {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.gt-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-xs);
}

.gt-hash {
  color: var(--primary);
  font-family: var(--font-mono, ui-monospace, monospace);
  font-weight: 600;
}

.gt-time {
  color: var(--muted);
}

/* hover 操作（默认隐藏，悬停浮现） */
.gt-ops {
  display: flex;
  align-items: center;
  gap: 4px;
  opacity: 0;
  transform: translateY(2px);
  transition: opacity var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.gt-node:hover .gt-ops,
.gt-node--head .gt-ops {
  opacity: 1;
  transform: translateY(0);
}
</style>
