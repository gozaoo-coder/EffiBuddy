<script setup lang="ts">
/**
 * SnapshotTimeline —— 会话版本管理「快照时间线」原子组件
 *
 * 职责（纯展示 + 轻交互，数据/操作由父组件下发）：
 * - 按时间倒序渲染快照节点：备注、相对时间、来源徽章（自动/手动/保护）、文件数/体积
 * - 每个节点提供「恢复」（回溯到该快照）与「删除」操作
 * - 最新一条标记为「当前」；带过渡动画的空态 / 加载态
 *
 * emits → 父组件：restore(id) / delete(id)
 */
import { Icon } from '../basic'
import type { SnapshotMeta } from '../../composables/useSnapshot'

const props = defineProps<{
  snapshots: SnapshotMeta[]
  loading: boolean
  /** 单节点 busy（精确到操作类型 + 快照 id） */
  busy: { type: 'restore' | 'delete'; id: string } | null
}>()

const emit = defineEmits<{
  (e: 'restore', id: string): void
  (e: 'delete', id: string): void
}>()

/** 相对时间（Unix 毫秒） */
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

/** 来源徽章文案 */
function sourceLabel(s: SnapshotMeta['source']): { text: string; cls: string } {
  if (s === 'auto') return { text: '自动', cls: 'auto' }
  if (s === 'manual') return { text: '手动', cls: 'manual' }
  return { text: '保护', cls: 'protect' }
}

/** 字节 → 可读体积 */
function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}
</script>

<template>
  <section class="st-card">
    <div class="st-head">
      <span class="st-title"><Icon name="git-commit" :size="15" />快照时间线</span>
      <span v-if="snapshots.length" class="st-count">{{ snapshots.length }} 条</span>
    </div>

    <!-- 加载中 -->
    <div v-if="loading && snapshots.length === 0" class="st-loading">
      <Icon name="loader" :size="16" class="st-spin" />
      <span>加载中…</span>
    </div>

    <!-- 空态 -->
    <div v-else-if="snapshots.length === 0" class="st-empty">
      <Icon name="history" :size="18" class="st-empty-icon" />
      <p>暂无快照。每次 edit / 写入文件后会自动生成，也可在上方手动保存。</p>
    </div>

    <!-- 时间线 -->
    <ol v-else class="st-list">
      <li
        v-for="(snap, i) in snapshots"
        :key="snap.id"
        class="st-node"
        :class="{ 'st-node--latest': i === 0 }"
      >
        <div class="st-rail">
          <span class="st-dot" :class="{ 'st-dot--latest': i === 0 }" />
          <span v-if="i !== snapshots.length - 1" class="st-line" />
        </div>
        <div class="st-body">
          <div class="st-node-head">
            <span class="st-msg">{{ snap.message }}</span>
            <span
              v-if="i === 0"
              class="st-badge st-badge--latest"
              title="这是最近一次快照"
            >当前</span>
            <span class="st-badge" :class="`st-badge--${sourceLabel(snap.source).cls}`">
              {{ sourceLabel(snap.source).text }}
            </span>
          </div>
          <div class="st-meta">
            <span>{{ relTime(snap.created_at) }}</span>
            <span class="st-meta-sep">·</span>
            <span>{{ snap.files }} 文件</span>
            <span class="st-meta-sep">·</span>
            <span>{{ fmtBytes(snap.bytes) }}</span>
          </div>
          <div class="st-ops">
            <button
              type="button"
              class="st-op st-op--restore"
              :disabled="busy !== null"
              @click="emit('restore', snap.id)"
            >
              <Icon
                :name="busy?.type === 'restore' && busy.id === snap.id ? 'loader' : 'restore'"
                :size="13"
                :class="{ 'st-spin': busy?.type === 'restore' && busy.id === snap.id }"
              />
              恢复
            </button>
            <button
              type="button"
              class="st-op st-op--delete"
              :disabled="busy !== null || i === 0"
              :title="i === 0 ? '不能删除最新一条快照' : '删除该快照'"
              @click="emit('delete', snap.id)"
            >
              <Icon
                :name="busy?.type === 'delete' && busy.id === snap.id ? 'loader' : 'delete'"
                :size="13"
                :class="{ 'st-spin': busy?.type === 'delete' && busy.id === snap.id }"
              />
              删除
            </button>
          </div>
        </div>
      </li>
    </ol>
  </section>
</template>

<style scoped>
.st-card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 10px 12px;
}

.st-head {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
}

.st-title {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.st-title :deep(.app-icon) {
  color: var(--primary);
}

.st-count {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* 加载 / 空态 */
.st-loading {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  color: var(--muted);
  padding: 12px 0;
}

.st-spin {
  animation: st-rotate 1s linear infinite;
}

@keyframes st-rotate {
  to {
    transform: rotate(360deg);
  }
}

.st-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 18px 8px;
  text-align: center;
}

.st-empty-icon {
  color: var(--muted);
  opacity: 0.7;
}

.st-empty p {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.6;
}

/* 时间线 */
.st-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
}

.st-node {
  display: flex;
  gap: 8px;
  padding: 4px 0;
}

/* 轨道 */
.st-rail {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 12px;
  flex-shrink: 0;
}

.st-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--border);
  margin-top: 4px;
  flex-shrink: 0;
}

.st-dot--latest {
  background: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 18%, transparent);
}

.st-line {
  flex: 1;
  width: 2px;
  background: var(--border);
  margin: 3px 0;
}

/* 节点内容 */
.st-body {
  flex: 1;
  min-width: 0;
  padding-bottom: 8px;
  border-bottom: 1px dashed var(--border);
}

.st-node:last-child .st-body {
  border-bottom: none;
  padding-bottom: 2px;
}

.st-node-head {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.st-msg {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.st-badge {
  font-size: var(--fs-xs);
  padding: 0 7px;
  border-radius: var(--radius-full);
  flex-shrink: 0;
}

.st-badge--latest {
  background: color-mix(in srgb, var(--primary) 16%, var(--card));
  color: var(--primary);
}

.st-badge--auto {
  background: color-mix(in srgb, var(--info) 14%, var(--card));
  color: var(--info);
}

.st-badge--manual {
  background: color-mix(in srgb, var(--success) 14%, var(--card));
  color: var(--success);
}

.st-badge--protect {
  background: color-mix(in srgb, var(--warn) 14%, var(--card));
  color: var(--warn);
}

.st-meta {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--fs-xs);
  color: var(--muted);
  margin-top: 2px;
}

.st-meta-sep {
  opacity: 0.5;
}

.st-ops {
  display: flex;
  gap: 6px;
  margin-top: 6px;
}

.st-op {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  font-size: var(--fs-xs);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg);
  color: var(--muted);
  cursor: pointer;
    transition: border-color 0.15s ease, color 0.15s ease, background 0.15s ease, opacity 0.15s ease;
}

.st-op:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.st-op--restore:hover:not(:disabled) {
  border-color: var(--primary);
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 8%, var(--card));
}

.st-op--delete:hover:not(:disabled) {
  border-color: var(--danger);
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 8%, var(--card));
}
</style>
