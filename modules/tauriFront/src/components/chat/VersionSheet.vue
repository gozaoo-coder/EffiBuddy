<script setup lang="ts">
/**
 * VersionSheet —— 会话版本管理（git 风格）
 *
 * 展示当前分支 + 全部版本引用（分支 / 临时版本 / 检查点）与当前分支提交历史。
 * 提供「检出」与「删除引用」操作；破坏性操作统一经 useVersioning 弹确认框。
 * 由 ChatComposer 底栏「版本」入口打开。
 */
import { inject } from 'vue'
import { Icon, BindSheet } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import type { VersionRefSummary } from '../../types'

const store = inject(CHAT_STORE_KEY)!
const { list, loading, sheetOpen, onCheckout, onDeleteRef } = store.versioning

/** 引用中文类别名 */
function refKindLabel(kind: string): string {
  switch (kind) {
    case 'main':
      return '主分支'
    case 'branch':
      return '分支'
    case 'temp':
      return '临时版本'
    case 'checkpoint':
      return '检查点'
    default:
      return kind
  }
}

/** 时间格式化 */
function fmtTime(ts: number): string {
  const d = new Date(ts)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

/** 提交类型标签 */
const kindLabelMap: Record<string, string> = {
  append: '消息',
  branch: '分支',
  temp_save: '临时版本',
  rollback: '回溯',
  undo: '撤回',
}

function kindLabel(kind: string): string {
  return kindLabelMap[kind] ?? kind
}
</script>

<template>
  <BindSheet v-model:visible="sheetOpen" title="会话版本管理" side="bottom" :height="'auto'">
    <div class="ver-sheet">
      <!-- 当前分支 + 刷新 -->
      <div class="ver-head">
        <div class="ver-head-label">当前分支</div>
        <div class="ver-head-row">
          <span class="ver-branch">{{ list?.head ?? 'main' }}</span>
          <button type="button" class="ver-refresh" title="刷新" @click="store.versioning.loadVersions()">
            <Icon name="refresh" :size="15" />
          </button>
        </div>
      </div>

      <div v-if="loading" class="ver-empty">加载中…</div>

      <template v-else>
        <!-- 版本引用列表（分支 / 临时版本 / 检查点） -->
        <div v-if="list && list.refs.length" class="ver-section">
          <div class="ver-section-title">版本引用</div>
          <div class="ver-ref-list">
            <div
              v-for="r in list.refs"
              :key="r.name"
              class="ver-ref"
              :class="{ 'ver-ref--current': r.name === list.head }"
            >
              <Icon
                :name="r.kind === 'temp' ? 'bookmark' : r.kind === 'checkpoint' ? 'clock' : 'branch'"
                :size="15"
                class="ver-ref-icon"
              />
              <div class="ver-ref-main">
                <div class="ver-ref-name">
                  {{ r.name }}
                  <span class="ver-ref-kind">{{ refKindLabel(r.kind) }}</span>
                  <span v-if="r.name === list.head" class="ver-ref-current">当前</span>
                </div>
                <div class="ver-ref-meta">
                  {{ fmtTime(r.created_at) }} · {{ r.message_count }} 条消息
                  <template v-if="r.note"> · {{ r.note }}</template>
                </div>
              </div>
              <div class="ver-ref-actions">
                <button
                  type="button"
                  class="ver-btn"
                  :disabled="r.name === list.head"
                  title="检出到此版本（临时版本/检查点会新建分支继续）"
                  @click="onCheckout(r.name)"
                >
                  <Icon name="history" :size="14" />
                  检出
                </button>
                <button
                  v-if="r.kind !== 'main' && r.name !== list.head"
                  type="button"
                  class="ver-btn ver-btn--danger"
                  title="删除此版本引用"
                  @click="onDeleteRef(r.name)"
                >
                  <Icon name="delete" :size="14" />
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- 当前分支提交历史（新 → 旧） -->
        <div v-if="list && list.commits.length" class="ver-section">
          <div class="ver-section-title">当前分支历史</div>
          <div class="ver-history">
            <div
              v-for="c in list.commits"
              :key="c.hash"
              class="ver-commit"
              :class="{ 'ver-commit--head': c.is_head }"
            >
              <div class="ver-commit-dot" :class="{ 'is-head': c.is_head }"></div>
              <div class="ver-commit-main">
                <div class="ver-commit-row">
                  <span class="ver-commit-kind">{{ kindLabel(c.kind) }}</span>
                  <span v-if="c.is_head" class="ver-commit-head">HEAD</span>
                </div>
                <div class="ver-commit-note">{{ c.note }}</div>
                <div class="ver-commit-meta">
                  {{ fmtTime(c.created_at) }} · {{ c.message_count }} 条 · {{ c.hash.slice(0, 10) }}…
                </div>
              </div>
            </div>
          </div>
        </div>

        <div v-if="!list || (!list.refs.length && !list.commits.length)" class="ver-empty">
          该会话还没有版本历史。每次发送/回复消息都会自动记录一个版本点，
          可随时在某条消息上开启分支、保存临时版本或回溯。
        </div>
      </template>
    </div>
  </BindSheet>
</template>

<style scoped>
.ver-sheet {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 4px 0 12px;
  max-height: 60vh;
  overflow-y: auto;
}

.ver-head {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 14px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.ver-head-label {
  font-size: 12px;
  color: var(--muted);
}

.ver-head-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.ver-branch {
  font-size: 15px;
  font-weight: 600;
  color: var(--primary);
  font-family: var(--font-mono, monospace);
}

.ver-refresh {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
}

.ver-refresh:hover {
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 10%, transparent);
}

.ver-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.ver-section-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
  padding: 0 2px;
}

.ver-ref-list,
.ver-history {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.ver-ref {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
}

.ver-ref--current {
  border-color: color-mix(in srgb, var(--primary) 40%, var(--border));
  background: color-mix(in srgb, var(--primary) 6%, var(--card));
}

.ver-ref-icon {
  color: var(--muted);
  flex-shrink: 0;
}

.ver-ref--current .ver-ref-icon {
  color: var(--primary);
}

.ver-ref-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.ver-ref-name {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  font-family: var(--font-mono, monospace);
}

.ver-ref-kind {
  font-size: 11px;
  font-weight: 400;
  color: var(--muted);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 1px 7px;
}

.ver-ref-current {
  font-size: 11px;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 12%, transparent);
  border-radius: var(--radius-full);
  padding: 1px 7px;
}

.ver-ref-meta {
  font-size: 12px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ver-ref-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.ver-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 9px;
  font-size: 12px;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 8%, transparent);
  border: 1px solid color-mix(in srgb, var(--primary) 26%, var(--border));
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease;
}

.ver-btn:hover:not(:disabled) {
  background: color-mix(in srgb, var(--primary) 16%, transparent);
  border-color: var(--primary);
}

.ver-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.ver-btn--danger {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 8%, transparent);
  border-color: color-mix(in srgb, var(--danger) 26%, var(--border));
  padding: 4px 7px;
}

.ver-btn--danger:hover:not(:disabled) {
  background: color-mix(in srgb, var(--danger) 14%, transparent);
  border-color: var(--danger);
}

.ver-history {
  padding-left: 4px;
}

.ver-commit {
  display: flex;
  gap: 10px;
  position: relative;
  padding: 6px 10px;
  border-radius: var(--radius-md);
}

.ver-commit:hover {
  background: var(--bg-2);
}

.ver-commit--head {
  background: color-mix(in srgb, var(--primary) 6%, var(--card));
}

.ver-commit-dot {
  width: 9px;
  height: 9px;
  margin-top: 5px;
  border-radius: 50%;
  background: var(--border);
  flex-shrink: 0;
}

.ver-commit-dot.is-head {
  background: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 20%, transparent);
}

.ver-commit-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.ver-commit-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.ver-commit-kind {
  font-size: 11px;
  color: var(--muted);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 1px 7px;
}

.ver-commit-head {
  font-size: 11px;
  color: var(--primary);
  font-weight: 700;
}

.ver-commit-note {
  font-size: 13px;
  color: var(--text);
}

.ver-commit-meta {
  font-size: 11px;
  color: var(--muted);
  font-family: var(--font-mono, monospace);
}

.ver-empty {
  font-size: 13px;
  color: var(--muted);
  line-height: 1.6;
  padding: 14px;
  text-align: center;
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
}
</style>
