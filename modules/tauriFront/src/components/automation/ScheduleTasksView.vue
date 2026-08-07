<script setup lang="ts">
/**
 * ScheduleTasksView 定时任务
 * 列出定时任务，支持创建 / 删除 / 启停开关，监听后端 "scheduled-task-result" 事件。
 * 纯内容视图（无 props / emits），由外部自动化容器承载。
 */
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  Button,
  Switch,
  Dropdown,
  Dialog,
  IconButton,
  Icon,
  useToast,
  type DropdownOption,
} from '../basic'
import type { ScheduledTask, ScheduledTaskResult, Skill } from '../../types'

const { toast } = useToast()

// ---------- 数据 ----------
const tasks = ref<ScheduledTask[]>([])
const skills = ref<Skill[]>([])
const loading = ref(false)

// 技能 id → 名称 映射，用于列表展示关联技能
const skillNameMap = computed(() => {
  const m = new Map<string, string>()
  for (const s of skills.value) m.set(s.id, s.name)
  return m
})

function skillNameOf(id: string): string {
  return skillNameMap.value.get(id) ?? '未知技能'
}

async function refresh() {
  loading.value = true
  try {
    const [t, s] = await Promise.all([
      invoke<ScheduledTask[]>('list_scheduled_tasks'),
      invoke<Skill[]>('list_skills'),
    ])
    tasks.value = t
    skills.value = s
  } catch (e) {
    toast({ content: `加载定时任务失败：${e}`, type: 'error' })
    tasks.value = []
    skills.value = []
  } finally {
    loading.value = false
  }
}

// ---------- cron 人类可读化 ----------
const dayMap: Record<string, string> = {
  '0': '日',
  '1': '一',
  '2': '二',
  '3': '三',
  '4': '四',
  '5': '五',
  '6': '六',
  '7': '日',
}

function pad2(n: string): string {
  if (n === '*' || n.startsWith('*')) return n
  const num = Number(n)
  if (Number.isNaN(num)) return n
  return num < 10 ? `0${num}` : `${num}`
}

function humanizeCron(cron: string): string {
  const parts = cron.trim().split(/\s+/)
  if (parts.length !== 5) return cron
  const [min, hour, dom, month, dow] = parts
  const hm = `${pad2(hour)}:${pad2(min)}`

  // 每小时整点：0 * * * *
  if (min === '0' && hour === '*' && dom === '*' && month === '*' && dow === '*') {
    return '每小时整点'
  }
  // 每天：0 9 * * *
  if (min !== '*' && hour !== '*' && dom === '*' && month === '*' && dow === '*') {
    return `每天 ${hm}`
  }
  // 每周：0 9 * * 1
  if (min !== '*' && hour !== '*' && dom === '*' && month === '*' && dow !== '*') {
    return `每周${dayMap[dow] ?? dow} ${hm}`
  }
  // 每 N 分钟：*/5 * * * *
  if (min.startsWith('*/') && hour === '*' && dom === '*' && month === '*' && dow === '*') {
    return `每 ${min.slice(2)} 分钟`
  }
  return cron
}

// ---------- 启停开关 ----------
async function onToggle(task: ScheduledTask, enabled: boolean) {
  // 乐观更新，失败回滚
  const prev = task.enabled
  task.enabled = enabled
  try {
    await invoke('toggle_scheduled_task', { id: task.id, enabled })
    toast({ content: enabled ? `已启用「${task.name}」` : `已停用「${task.name}」`, type: 'success' })
  } catch (e) {
    task.enabled = prev
    toast({ content: `操作失败：${e}`, type: 'error' })
  }
}

// ---------- 删除 ----------
const deleteDialogOpen = ref(false)
const deleteTarget = ref<ScheduledTask | null>(null)

function askDelete(task: ScheduledTask) {
  deleteTarget.value = task
  deleteDialogOpen.value = true
}

async function confirmDelete() {
  const t = deleteTarget.value
  if (!t) return
  try {
    await invoke('delete_scheduled_task', { id: t.id })
    toast({ content: `已删除任务「${t.name}」`, type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
  } finally {
    deleteTarget.value = null
  }
}

// ---------- 新建 Dialog ----------
const dialogOpen = ref(false)
const draft = ref({
  name: '',
  skill_id: '',
  cron: '0 9 * * *',
})

const skillDropdownOptions = computed<DropdownOption[]>(() =>
  skills.value.map((s) => ({ label: s.name, value: s.id, icon: s.builtin ? 'star' : 'spark' })),
)

function onSkillChange(v: string | number, _opt: DropdownOption) {
  draft.value.skill_id = String(v)
}

const cronPresets: { label: string; cron: string }[] = [
  { label: '每天9点', cron: '0 9 * * *' },
  { label: '每小时', cron: '0 * * * *' },
  { label: '每周一9点', cron: '0 9 * * 1' },
]

function applyPreset(cron: string) {
  draft.value.cron = cron
}

function openCreate() {
  draft.value = { name: '', skill_id: '', cron: '0 9 * * *' }
  dialogOpen.value = true
}

async function confirmCreate() {
  const name = draft.value.name.trim()
  if (!name) {
    toast({ content: '请输入任务名称', type: 'warn' })
    return
  }
  if (!draft.value.skill_id) {
    toast({ content: '请选择关联技能', type: 'warn' })
    return
  }
  const cronParts = draft.value.cron.trim().split(/\s+/)
  if (cronParts.length !== 5) {
    toast({ content: 'cron 表达式需为 5 字段（分 时 日 月 周）', type: 'warn' })
    return
  }
  try {
    const task: ScheduledTask = {
      id: '',
      name,
      skill_id: draft.value.skill_id,
      cron: draft.value.cron.trim(),
      enabled: true,
      created_at: 0,
      last_run: null,
    }
    await invoke('create_scheduled_task', { task })
    toast({ content: `已创建任务「${name}」`, type: 'success' })
    dialogOpen.value = false
    await refresh()
  } catch (e) {
    toast({ content: `创建失败：${e}`, type: 'error' })
  }
}

// ---------- 时间格式化 ----------
function formatRelativeTime(ts: number | null | undefined): string {
  if (!ts) return '尚未执行'
  const diff = Date.now() - ts
  if (diff < 60000) return '刚刚'
  if (diff < 3600000) return `${Math.floor(diff / 60000)} 分钟前`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)} 小时前`
  if (diff < 2592000000) return `${Math.floor(diff / 86400000)} 天前`
  try {
    return new Date(ts).toLocaleString()
  } catch {
    return ''
  }
}

// ---------- 监听后端 scheduled-task-result 事件 ----------
let unlistens: UnlistenFn[] = []

async function setupListeners() {
  unlistens.push(
    await listen<ScheduledTaskResult>('scheduled-task-result', (e) => {
      const p = e.payload
      const head = `定时任务「${p.task_name}」执行${p.success ? '完成' : '失败'}`
      const preview = p.content.length > 80 ? p.content.slice(0, 80) + '…' : p.content
      toast({
        content: `${head}：${preview}`,
        type: p.success ? 'success' : 'error',
        duration: p.success ? 4000 : 0,
      })
      // 刷新列表以更新 last_run
      refresh()
    }),
  )
}

onMounted(() => {
  refresh()
  setupListeners()
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
})
</script>

<template>
  <div class="sched-body">
    <!-- 顶部说明 -->
    <header class="sched-hero">
      <div class="hero-mark"><Icon name="alarm" :size="28" /></div>
      <div class="hero-text">
        <h2 class="hero-title">定时任务</h2>
        <p class="hero-sub">按 cron 表达式定时触发技能，结果会以通知形式推送</p>
      </div>
    </header>

    <!-- 任务列表 -->
    <section class="section">
      <div class="section-head">
        <span class="section-title">任务列表</span>
        <span v-if="tasks.length" class="count-badge">{{ tasks.length }}</span>
      </div>

      <div v-if="!tasks.length && !loading" class="empty-state">
        <div class="empty-illust"><Icon name="alarm" :size="48" /></div>
        <p class="empty-text">还没有定时任务</p>
        <p class="empty-hint">点击下方按钮，创建你的第一个定时任务</p>
      </div>

      <div v-else class="task-list">
        <div
          v-for="t in tasks"
          :key="t.id"
          class="task-card"
          :class="{ disabled: !t.enabled }"
        >
          <div class="task-card-main">
            <div class="task-top">
              <span class="task-name">{{ t.name }}</span>
              <span class="task-cron">{{ humanizeCron(t.cron) }}</span>
            </div>
            <div class="task-meta">
              <span class="task-skill"><Icon name="bolt" :size="16" /> {{ skillNameOf(t.skill_id) }}</span>
              <span class="task-last">上次：{{ formatRelativeTime(t.last_run) }}</span>
            </div>
          </div>
          <div class="task-card-actions">
            <Switch
              :model-value="t.enabled"
              size="sm"
              @update:model-value="(v: boolean) => onToggle(t, v)"
            />
            <IconButton
              size="sm"
              title="删除"
              @click="askDelete(t)"
            ><Icon name="delete" :size="18" /></IconButton>
          </div>
        </div>
      </div>
    </section>

    <!-- 底部新建按钮 -->
    <div class="sched-footer">
      <Button variant="primary" block @click="openCreate">
        <template #icon><Icon name="plus" :size="18" /></template>
        新建任务
      </Button>
    </div>

    <!-- 新建 Dialog -->
    <Dialog
      v-model:visible="dialogOpen"
      title="新建定时任务"
      confirm-text="创建"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmCreate"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">任务名称</label>
          <input
            v-model="draft.name"
            type="text"
            class="field-input"
            placeholder="例如：每日早报"
          />
        </div>
        <div class="field">
          <label class="field-label">关联技能</label>
          <Dropdown
            :model-value="draft.skill_id"
            :options="skillDropdownOptions"
            :searchable="true"
            placeholder="选择技能..."
            size="md"
            @change="onSkillChange"
          />
        </div>
        <div class="field">
          <label class="field-label">cron 表达式</label>
          <input
            v-model="draft.cron"
            type="text"
            class="field-input mono"
            placeholder="分 时 日 月 周（如 0 9 * * *）"
          />
          <div class="cron-presets">
            <button
              v-for="p in cronPresets"
              :key="p.cron"
              type="button"
              class="preset-btn"
              :class="{ active: draft.cron === p.cron }"
              @click="applyPreset(p.cron)"
            >{{ p.label }}</button>
          </div>
          <p class="field-hint">预览：{{ humanizeCron(draft.cron) }}</p>
        </div>
      </div>
    </Dialog>

    <!-- 删除确认 -->
    <Dialog
      v-model:visible="deleteDialogOpen"
      title="删除定时任务"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-delete-content">
        确定删除任务「{{ deleteTarget?.name }}」？此操作不可撤销。
      </div>
    </Dialog>
  </div>
</template>

<style scoped>
.sched-body {
  padding: 20px 24px 32px;
  display: flex;
  flex-direction: column;
  gap: 20px;
  overflow-y: auto;
}

/* ---------- 顶部说明 ---------- */
.sched-hero {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 18px 20px;
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.14), rgba(74, 126, 255, 0.02));
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.hero-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  border-radius: var(--radius-lg);
  background: var(--card-2);
  font-size: 22px;
  flex-shrink: 0;
}

.hero-text {
  min-width: 0;
}

.hero-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text);
}

.hero-sub {
  margin: 4px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 分区 ---------- */
.section {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.section-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.section-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.count-badge {
  padding: 2px 10px;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
}

/* ---------- 任务卡片 ---------- */
.task-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.task-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  transition: border-color var(--duration-fast) var(--ease-standard),
    opacity var(--duration-fast) var(--ease-standard);
}

.task-card:hover {
  border-color: var(--primary);
}

.task-card.disabled {
  opacity: 0.6;
}

.task-card-main {
  flex: 1;
  min-width: 0;
}

.task-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.task-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.task-cron {
  flex-shrink: 0;
  padding: 1px 8px;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(74, 126, 255, 0.1);
  border-radius: var(--radius-full);
}

.task-meta {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 4px;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.task-skill {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.task-card-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* ---------- 空状态 ---------- */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 36px 20px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  text-align: center;
}

.empty-illust {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  border-radius: 50%;
  background: var(--card-2);
  color: var(--muted);
  font-size: 26px;
  margin-bottom: 4px;
}

.empty-text {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.empty-hint {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
  max-width: 320px;
}

/* ---------- 底部 ---------- */
.sched-footer {
  padding-top: 4px;
}

/* ---------- Dialog 表单 ---------- */
.form-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 4px 0;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.field-label {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text);
}

.field-input {
  width: 100%;
  height: var(--h-control-md);
  padding: 0 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.field-input:focus {
  border-color: var(--primary);
}

.field-input.mono {
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.field-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.cron-presets {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.preset-btn {
  padding: 4px 12px;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--text);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.preset-btn:hover {
  border-color: var(--primary);
  color: var(--primary);
}

.preset-btn.active {
  background: var(--primary);
  border-color: var(--primary);
  color: #fff;
}

.dialog-delete-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>