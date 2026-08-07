<script setup lang="ts">
/**
 * UserTodoPage —— 「我的待办」示例插件页面
 *
 * 作为「插件注册页签/页面/组件」体系的实例：
 * - 页面元数据由后端贡献注册（id = effisuite/user-todo，entry=builtin）
 * - 本组件在前端页面注册表（usePluginPages）按 id 解析渲染
 *
 * 功能：
 * - 添加 / 完成 / 删除待办，筛选（全部 / 进行中 / 已完成）
 * - 待办列表 localStorage 持久化（示例页面自身状态）
 * - 「自动清理已完成」偏好通过插件配置系统（set_plugin_config）存入 appdata，
 *   演示「插件请求配置系统存储到应用 appdata」的能力
 */
import { ref, computed, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from '../Icon.vue'
import { useToast } from '../basic'

interface TodoItem {
  id: string
  text: string
  done: boolean
  createdAt: number
}

// 本页面归属的「插件 id」与「配置键」
const PLUGIN_ID = 'effisuite/user-todo'
const CFG_AUTO_CLEAN = 'autoCleanDone'

const { toast } = useToast()

const todos = ref<TodoItem[]>([])
const draft = ref('')
const filter = ref<'all' | 'active' | 'done'>('all')
const autoClean = ref(false)

// ---------- 待办列表持久化（localStorage） ----------
const STORAGE_KEY = 'effisuite:user-todo:todos'

function loadTodos(): TodoItem[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? (JSON.parse(raw) as TodoItem[]) : []
  } catch {
    return []
  }
}

function saveTodos() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(todos.value))
  } catch {
    /* ignore */
  }
}

// ---------- 插件配置系统（appdata） ----------
async function loadPluginConfig() {
  try {
    const v = await invoke<boolean | null>('get_plugin_config', {
      pluginId: PLUGIN_ID,
      key: CFG_AUTO_CLEAN,
    })
    if (v !== null) autoClean.value = !!v
  } catch {
    autoClean.value = false
  }
}

async function saveAutoClean(v: boolean) {
  autoClean.value = v
  try {
    await invoke('set_plugin_config', {
      pluginId: PLUGIN_ID,
      key: CFG_AUTO_CLEAN,
      value: v,
    })
    toast({ content: v ? '已开启自动清理已完成' : '已关闭自动清理', type: 'success' })
  } catch (e) {
    toast({ content: `保存插件配置失败：${e}`, type: 'error' })
  }
}

// ---------- 待办操作 ----------
function addTodo() {
  const text = draft.value.trim()
  if (!text) {
    toast({ content: '请输入待办内容', type: 'warn' })
    return
  }
  todos.value.unshift({
    id: `t_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
    text,
    done: false,
    createdAt: Date.now(),
  })
  draft.value = ''
  saveTodos()
}

function toggleTodo(item: TodoItem) {
  item.done = !item.done
  saveTodos()
}

function removeTodo(item: TodoItem) {
  todos.value = todos.value.filter((t) => t.id !== item.id)
  saveTodos()
}

function removeDone() {
  const n = todos.value.filter((t) => t.done).length
  todos.value = todos.value.filter((t) => !t.done)
  saveTodos()
  toast({ content: n ? `已清除 ${n} 条已完成` : '没有已完成的待办', type: n ? 'success' : 'warn' })
}

const filteredTodos = computed(() => {
  if (filter.value === 'active') return todos.value.filter((t) => !t.done)
  if (filter.value === 'done') return todos.value.filter((t) => t.done)
  return todos.value
})

const stats = computed(() => {
  const total = todos.value.length
  const done = todos.value.filter((t) => t.done).length
  return { total, done, active: total - done }
})

watch(
  () => autoClean.value,
  (v) => {
    if (v) removeDone()
  },
)

onMounted(() => {
  todos.value = loadTodos()
  void loadPluginConfig()
})
</script>

<template>
  <div class="todo-page">
    <!-- 头部 -->
    <header class="todo-hero">
      <div class="todo-hero-main">
        <h2 class="todo-title">我的待办</h2>
        <p class="todo-sub">由插件注册表提供的示例页面 · 数据保存在本地</p>
      </div>
      <div class="todo-stats">
        <div class="stat">
          <span class="stat-num">{{ stats.active }}</span>
          <span class="stat-label">进行中</span>
        </div>
        <div class="stat">
          <span class="stat-num">{{ stats.done }}</span>
          <span class="stat-label">已完成</span>
        </div>
      </div>
    </header>

    <!-- 输入区 -->
    <div class="todo-input-row">
      <input
        v-model="draft"
        type="text"
        class="todo-input"
        placeholder="输入待办事项，回车添加…"
        @keyup.enter="addTodo"
      />
      <button type="button" class="todo-add" @click="addTodo">
        <Icon name="plus" :size="16" />
        添加
      </button>
    </div>

    <!-- 工具栏 -->
    <div class="todo-toolbar">
      <div class="todo-filters">
        <button
          v-for="f in [
            { key: 'all', label: '全部' },
            { key: 'active', label: '进行中' },
            { key: 'done', label: '已完成' },
          ]"
          :key="f.key"
          type="button"
          class="todo-filter"
          :class="{ active: filter === f.key }"
          @click="filter = f.key as 'all' | 'active' | 'done'"
        >
          {{ f.label }}
        </button>
      </div>
      <div class="todo-toolbar-right">
        <label class="todo-auto-clean" title="开启后勾选完成的待办会自动移除">
          <input v-model="autoClean" type="checkbox" @change="saveAutoClean(autoClean)" />
          <span>自动清理已完成</span>
        </label>
        <button v-if="stats.done > 0" type="button" class="todo-clear" @click="removeDone">
          清除已完成
        </button>
      </div>
    </div>

    <!-- 列表 -->
    <div class="todo-list">
      <TransitionGroup name="todo" tag="div" class="todo-items">
        <div v-for="item in filteredTodos" :key="item.id" class="todo-item" :class="{ done: item.done }">
          <button
            type="button"
            class="todo-check"
            :class="{ checked: item.done }"
            :aria-label="item.done ? '标记为未完成' : '标记为已完成'"
            @click="toggleTodo(item)"
          >
            <Icon v-if="item.done" name="check" :size="13" />
          </button>
          <span class="todo-text">{{ item.text }}</span>
          <button type="button" class="todo-remove" title="删除" @click="removeTodo(item)">
            <Icon name="close" :size="14" />
          </button>
        </div>
      </TransitionGroup>

      <div v-if="filteredTodos.length === 0" class="todo-empty">
        <span class="todo-empty-icon"><Icon name="book" :size="40" /></span>
        <p class="todo-empty-text">
          {{ stats.total === 0 ? '还没有待办，先添加一条吧' : '当前筛选下没有待办' }}
        </p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.todo-page {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  max-width: 720px;
  margin: 0 auto;
  padding: 32px 24px 40px;
  overflow-y: auto;
}

/* 头部 */
.todo-hero {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
  margin-bottom: 22px;
}

.todo-title {
  margin: 0;
  font-size: 26px;
  font-weight: 700;
  letter-spacing: -0.4px;
  color: var(--text);
}

.todo-sub {
  margin: 6px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
}

.todo-stats {
  display: flex;
  gap: 10px;
}

.stat {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 8px 14px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.stat-num {
  font-size: 20px;
  font-weight: 700;
  color: var(--primary);
}

.stat-label {
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* 输入区 */
.todo-input-row {
  display: flex;
  gap: 10px;
}

.todo-input {
  flex: 1;
  height: 44px;
  padding: 0 16px;
  font-family: inherit;
  font-size: var(--fs-base);
  color: var(--text);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  outline: none;
  transition: border-color var(--duration-fast), box-shadow var(--duration-fast);
}

.todo-input:focus {
  border-color: var(--primary);
  box-shadow: 0 0 0 3px rgba(74, 126, 255, 0.15);
}

.todo-input::placeholder {
  color: var(--muted);
}

.todo-add {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 44px;
  padding: 0 20px;
  border: none;
  border-radius: var(--radius-lg);
  background: var(--primary);
  color: #fff;
  font-size: var(--fs-base);
  font-weight: 500;
  cursor: pointer;
  transition: opacity var(--duration-fast);
  flex-shrink: 0;
}

.todo-add:hover {
  opacity: 0.9;
}

/* 工具栏 */
.todo-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin: 18px 0 12px;
  flex-wrap: wrap;
}

.todo-filters {
  display: flex;
  gap: 4px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 3px;
}

.todo-filter {
  padding: 5px 14px;
  border: none;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--muted);
  font-size: var(--fs-sm);
  cursor: pointer;
  transition: background var(--duration-fast), color var(--duration-fast);
}

.todo-filter:hover {
  color: var(--text);
}

.todo-filter.active {
  background: var(--primary);
  color: #fff;
}

.todo-toolbar-right {
  display: flex;
  align-items: center;
  gap: 14px;
}

.todo-auto-clean {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--muted);
  cursor: pointer;
  user-select: none;
}

.todo-auto-clean input {
  accent-color: var(--primary);
}

.todo-clear {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--danger);
  font-size: var(--fs-xs);
  cursor: pointer;
}

.todo-clear:hover {
  text-decoration: underline;
}

/* 列表 */
.todo-list {
  flex: 1;
  min-height: 0;
}

.todo-items {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.todo-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  transition: border-color var(--duration-fast), background var(--duration-fast),
    opacity var(--duration-fast);
}

.todo-item:hover {
  border-color: var(--primary);
}

.todo-item.done {
  opacity: 0.6;
}

.todo-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  border: 1.5px solid var(--border);
  background: transparent;
  color: transparent;
  flex-shrink: 0;
  cursor: pointer;
  transition: background var(--duration-fast), border-color var(--duration-fast),
    color var(--duration-fast);
}

.todo-check.checked {
  background: var(--primary);
  border-color: var(--primary);
  color: #fff;
}

.todo-text {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-base);
  color: var(--text);
  word-break: break-word;
}

.todo-item.done .todo-text {
  text-decoration: line-through;
  color: var(--muted);
}

.todo-remove {
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
  flex-shrink: 0;
  opacity: 0;
  transition: opacity var(--duration-fast), background var(--duration-fast), color var(--duration-fast);
}

.todo-item:hover .todo-remove {
  opacity: 1;
}

.todo-remove:hover {
  background: var(--card-2);
  color: var(--danger);
}

/* 空状态 */
.todo-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 48px 20px;
  text-align: center;
}

.todo-empty-icon {
  display: inline-flex;
  color: var(--muted);
  opacity: 0.5;
}

.todo-empty-text {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
}

/* 列表项过渡 */
.todo-enter-active,
.todo-leave-active {
    transition: opacity var(--duration-base) var(--ease-standard),
      transform var(--duration-base) var(--ease-standard);
}

.todo-enter-from,
.todo-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}

.todo-leave-active {
  position: absolute;
}
</style>
