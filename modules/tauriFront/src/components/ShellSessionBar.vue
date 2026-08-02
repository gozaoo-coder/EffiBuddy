<script setup lang="ts">
/**
 * ShellSessionBar —— main-content 底栏的命令会话便签栏
 *
 * 展示 AI 启用的后台命令会话（shell_session_start/send/read 工具创建）的实时工作状态：
 * - 位于聊天主区（.chat-main）底部，允许折叠/展开
 * - 折叠态：一条细栏，可看到正在活跃运行的会话（#短ID 便签 chip，带呼吸点）
 * - 展开态：每个会话一张「便签」卡片，头部短 ID + 名称 + 运行态，正文为等宽日志
 * - 会话事件经 shell-session-event 推送（含 conversation_id），按当前会话过滤
 * - 支持复制日志、手动结束（kill）会话
 *
 * 样式遵循应用白/灰/黑三色调 + design tokens；便签用「ID 派生色」的顶部标签
 * 区分多个会话，正文保持中性便于阅读。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import Icon from './Icon.vue'
import type { ShellSessionEventPayload, ShellSessionInfo, ShellSessionRecord } from '../types'

const props = defineProps<{
  /** 当前对话 conversation_id；保留字段（当前展示全局会话坞，暂不按会话过滤） */
  conversationId?: string | null
  /** 折叠状态（受控，由 composer-meta 按钮 / 头部点击切换） */
  expanded: boolean
}>()

const emit = defineEmits<{
  (e: 'update:expanded', v: boolean): void
  (e: 'running-count', n: number): void
}>()

// ---------- 状态 ----------
/** session_id → 会话记录 */
const records = ref<Record<string, ShellSessionRecord>>({})
const lastTick = ref(0)

const sorted = computed(() => {
  const list = Object.values(records.value)
  list.sort((a, b) => b.last_active - a.last_active)
  return list
})
const runningCount = computed(() => sorted.value.filter((s) => s.running).length)
const hasAny = computed(() => sorted.value.length > 0)

// ---------- 工具 ----------
function ensure(id: string, seed: Partial<ShellSessionRecord> = {}): ShellSessionRecord {
  let r = records.value[id]
  if (!r) {
    r = {
      id,
      name: `会话 #${id}`,
      shell: 'cmd',
      cwd: '',
      running: true,
      last_command: '',
      lines: [],
      last_active: Date.now(),
      ...seed,
    }
    records.value[id] = r
  }
  return r
}

/** 会话 ID 派生一个稳定 pastel 色，用于便签顶部标签区分 */
function tagColor(id: string): string {
  let h = 0
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) % 360
  return `hsl(${h}, 45%, 55%)`
}

function appendLines(r: ShellSessionRecord, kind: 'cmd' | 'out' | 'err' | 'info', text: string) {
  const lines = text.split('\n')
  for (const t of lines) {
    if (!t) continue
    r.lines.push({ kind, text: t })
  }
  // 日志行数上限：防止长任务刷爆内存
  if (r.lines.length > 2000) {
    r.lines.splice(0, r.lines.length - 2000)
  }
  r.last_active = Date.now()
}

async function scrollLogs() {
  await nextTick()
  const logs = Array.from(document.querySelectorAll<HTMLElement>('.ss-log'))
  for (const el of logs) el.scrollTop = el.scrollHeight
}

// ---------- 事件 ----------
let unlisten: UnlistenFn | null = null

async function onEvent(p: ShellSessionEventPayload) {
  // 会话是全局共享的（后端 ShellSessionManager 为单例），底栏作为「AI 工作状态」全局坞展示全部会话；
  // 事件里携带的 conversation_id 仅保留供将来按会话聚焦使用。
  void props.conversationId
  const r = ensure(p.session_id)
  switch (p.kind) {
    case 'started':
      // content 形如 "cmd · 名称 · cwd(4 位)"
      r.running = true
      if (p.content && r.name === `会话 #${p.session_id}`) {
        r.name = p.content.split('·')[0]?.trim() || r.name
      }
      appendLines(r, 'info', `▸ 会话启动（${p.content}）`)
      break
    case 'command':
      r.running = true
      r.last_command = p.content
      appendLines(r, 'cmd', `> ${p.content}`)
      break
    case 'output':
      appendLines(r, p.is_error ? 'err' : 'out', p.content)
      break
    case 'exited':
      r.running = false
      appendLines(r, 'info', `■ 进程已退出（code ${p.content}）`)
      break
    case 'error':
      appendLines(r, 'err', p.content)
      break
  }
  lastTick.value = Date.now()
  void scrollLogs()
}

async function refreshFromBackend() {
  try {
    const list = await invoke<ShellSessionInfo[]>('list_shell_sessions')
    for (const info of list) {
      const r = ensure(info.id, {
        name: info.name,
        shell: info.shell,
        cwd: info.cwd,
        running: info.running,
        last_command: info.last_command,
        last_active: info.last_active,
      })
      r.shell = info.shell
      r.cwd = info.cwd
      r.running = info.running
      r.last_command = info.last_command
      r.last_active = info.last_active
    }
  } catch {
    // 后端命令不可用时静默降级（仅展示实时事件）
  }
}

// ---------- 用户操作 ----------
function toggleExpanded() {
  emit('update:expanded', !props.expanded)
}

async function copyLog(id: string) {
  const r = records.value[id]
  if (!r) return
  const text = r.lines.map((l) => l.text).join('\n')
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    /* ignore */
  }
}

async function killSession(id: string) {
  const r = records.value[id]
  if (!r) return
  try {
    await invoke('kill_shell_session', { sessionId: id })
  } catch {
    /* ignore */
  }
  r.running = false
  appendLines(r, 'info', '■ 已请求结束会话')
  lastTick.value = Date.now()
}

// 运行中会话数变化时上报(供 composer-meta 徽标展示)
watch(runningCount, (n) => emit('running-count', n), { immediate: true })

onMounted(async () => {
  await refreshFromBackend()
  unlisten = await listen<ShellSessionEventPayload>('shell-session-event', (e) => onEvent(e.payload))
})

onUnmounted(() => {
  unlisten?.()
  unlisten = null
})
</script>

<template>
  <!-- 折叠时整条不渲染（组件保持挂载以持续接收 shell-session-event，数据不丢）；
       展开/折叠由 composer-meta 按钮控制，点击头部也可折叠 -->
  <div v-if="expanded" class="ss-bar">
    <!-- 头部：展示状态；点击折叠 -->
    <div class="ss-head" title="折叠命令会话" @click="toggleExpanded">
      <Icon name="keyboard" :size="14" />
      <span class="ss-head-title">命令会话</span>
      <span v-if="runningCount" class="ss-head-count ss-head-count--running">
        <span class="ss-dot" />{{ runningCount }} 运行中
      </span>
      <span v-else-if="hasAny" class="ss-head-count">{{ sorted.length }} 个</span>
      <span v-else class="ss-head-idle">空闲</span>
    </div>

    <!-- 便签卡片堆叠 -->
    <div class="ss-notes">
      <div v-for="s in sorted" :key="s.id" class="ss-note" :class="{ 'ss-note--dead': !s.running }">
        <div class="ss-note-tag" :style="{ background: tagColor(s.id) }" />
        <div class="ss-note-head">
          <span class="ss-note-id">#{{ s.id }}</span>
          <span class="ss-note-name" :title="s.cwd">{{ s.name }}</span>
          <span class="ss-note-shell">{{ s.shell }}</span>
          <span class="ss-note-status" :class="{ running: s.running }">
            <span class="ss-dot" :class="{ 'ss-dot--pulse': s.running }" />
            {{ s.running ? '运行中' : '已退出' }}
          </span>
          <span class="ss-note-spacer" />
          <button class="ss-note-act" title="复制日志" @click.stop="copyLog(s.id)">
            <Icon name="copy" :size="13" />
          </button>
          <button class="ss-note-act ss-note-act--kill" title="结束会话" @click.stop="killSession(s.id)">
            <Icon name="close" :size="13" />
          </button>
        </div>
          <div class="ss-log">
          <div
            v-for="(ln, i) in s.lines"
            :key="i"
            class="ss-log-line"
            :class="`ss-log-line--${ln.kind}`"
          >{{ ln.text }}</div>
          <div v-if="!s.lines.length" class="ss-log-empty">等待输出…</div>
        </div>
      </div>
      <div v-if="!hasAny" class="ss-notes-empty">
        <Icon name="keyboard" :size="20" />
        <span>还没有命令会话。让 AI 执行 shell_session_start 后，这里会实时显示它的工作状态。</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ss-bar {
  flex-shrink: 0;
  border-top: 1px solid var(--border, #e5e7eb);
  background: var(--bg, #ffffff);
  overflow: hidden;
  user-select: none;
}

.ss-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 14px;
  cursor: pointer;
  color: var(--muted, #6b7280);
  font-size: 12px;
  transition: background 0.15s ease;
}

.ss-head:hover {
  background: var(--card-2, #f3f4f6);
}

.ss-head-title {
  font-weight: 600;
  color: var(--text, #111827);
}

.ss-head-count {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 1px 8px;
  border-radius: 999px;
  font-size: 11px;
  background: var(--card-2, #f3f4f6);
  color: var(--muted, #6b7280);
}

.ss-head-count--running {
  background: color-mix(in srgb, var(--success, #10a37f) 14%, var(--card, #fff));
  color: var(--success, #10a37f);
}

.ss-head-idle {
  font-size: 11px;
  color: var(--muted, #9ca3af);
}

.ss-head-spacer,
.ss-note-spacer {
  flex: 1;
}

/* ---------- 呼吸点 ---------- */
.ss-dot {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success, #10a37f);
}

.ss-dot--pulse {
  animation: ss-pulse 1.6s ease-in-out infinite;
}

@keyframes ss-pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.35; transform: scale(0.8); }
}

/* ---------- 便签 ---------- */
.ss-notes {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 0 14px 12px;
  max-height: 46vh;
  overflow-y: auto;
}

.ss-note {
  position: relative;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border, #e5e7eb);
  border-radius: var(--radius-md, 12px);
  background: var(--card, #fff);
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.04);
  overflow: hidden;
}

.ss-note--dead {
  opacity: 0.72;
}

.ss-note-tag {
  height: 3px;
}

.ss-note-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 12px;
  border-bottom: 1px solid var(--border, #e5e7eb);
  font-size: 12px;
}

.ss-note-id {
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-weight: 700;
  color: var(--primary, #4a7eff);
}

.ss-note-name {
  font-weight: 600;
  color: var(--text, #111827);
  max-width: 240px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ss-note-shell {
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 10px;
  background: var(--card-2, #f3f4f6);
  color: var(--muted, #6b7280);
}

.ss-note-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--muted, #6b7280);
}

.ss-note-status.running {
  color: var(--success, #10a37f);
}

.ss-note-act {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--muted, #6b7280);
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.ss-note-act:hover {
  background: var(--card-2, #f3f4f6);
  color: var(--text, #111827);
}

.ss-note-act--kill:hover {
  background: color-mix(in srgb, var(--danger, #ef4444) 12%, transparent);
  color: var(--danger, #ef4444);
}

/* ---------- 日志 ---------- */
.ss-log {
  max-height: 180px;
  overflow-y: auto;
  padding: 8px 12px;
  background: #0f1115;
  font-family: ui-monospace, SFMono-Regular, Consolas, 'Courier New', monospace;
  font-size: 11.5px;
  line-height: 1.55;
}

.ss-log-line {
  white-space: pre-wrap;
  word-break: break-all;
  color: #d1d5db;
}

.ss-log-line--cmd {
  color: #7dd3fc;
  font-weight: 600;
}

.ss-log-line--err {
  color: #fca5a5;
}

.ss-log-line--info {
  color: #9ca3af;
  font-style: italic;
}

.ss-log-empty {
  color: #6b7280;
  font-style: italic;
}

.ss-notes-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 24px 16px;
  color: var(--muted, #9ca3af);
  font-size: 12px;
  text-align: center;
}
</style>
