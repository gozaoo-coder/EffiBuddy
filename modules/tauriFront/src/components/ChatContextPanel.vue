<script setup lang="ts">
/**
 * ChatContextPanel —— 聊天主内容右栏
 *
 * 两个页签：
 * 1. 概览（Overview）：todoTree 卡片 / 上下文窗口可视化 / 用量指标 / 用量分析
 * 2. 压缩上下文（Compressed Context）：压缩机制设置（阈值可调）+ 压缩决策列表
 *
 * 数据来源：
 * - todoTree：`get_todo_tree` / `save_todo_tree` / `clear_todo_tree` + `todo-tree-updated` 事件
 * - 压缩状态：`get_compression_state` / `clear_compression_state` / `compress_messages_stream`
 * - 压缩设置：`get_config` / `set_config`
 */
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Icon, Button, IconButton, Switch, Slider, useToast } from './basic'
import TodoTreeNode from './todo/TodoTreeNode.vue'
import type {
  Message,
  TodoItem,
  TodoNode,
    TodoStatus,
  CompressionState,
  CompressionAction,
  CompressionSettings,
  AgentConfig,
} from '../types'

const props = defineProps<{
  /** 当前会话 id（null 表示未建立会话） */
  conversationId?: string | null
  /** 当前会话消息列表（用量统计用） */
  messages?: Message[]
  /** 上下文窗口：已用 token 估算值 */
  contextUsedTokens?: number
  /** 上下文窗口：模型最大 token 上限 */
  contextMaxTokens?: number
  /** 当前激活模型计费单价（元/百万 tokens）；未配置时为 null */
  pricing?: {
    cache_hit_per_m: number
    cache_miss_per_m: number
    output_per_m: number
  } | null
  /** 是否正在流式发送中（压缩页签按钮态用） */
  streaming?: boolean
}>()

const { toast } = useToast()

// ===================== 页签 =====================
type PanelTab = 'overview' | 'compress'
const activeTab = ref<PanelTab>('overview')

// ===================== todoTree =====================
const todoItems = ref<TodoItem[]>([])
const todoLoaded = ref(false)
/** 编辑态：key = 'add-root' | `edit-${id}` | `add-child-${parentId}`，value = 草稿 TodoItem */
const editing = ref<Record<string, TodoItem>>({})

/** 把扁平 TodoItem 还原为树 */
function buildTree(items: TodoItem[]): TodoNode[] {
  const nodes: TodoNode[] = items.map((t) => ({
    id: t.id,
    content: t.content,
    priority: t.priority,
    status: t.status,
    summary: t.summary,
    children: [],
  }))
  const index = new Map<string, TodoNode>()
  nodes.forEach((n) => index.set(n.id, n))
  const roots: TodoNode[] = []
  for (const t of items) {
    const node = index.get(t.id)!
    const pid = t.parent_id
    if (pid && index.has(pid) && pid !== t.id) {
      index.get(pid)!.children.push(node)
    } else {
      roots.push(node)
    }
  }
  return roots
}
const todoTree = computed(() => buildTree(todoItems.value))

async function loadTodos() {
  todoLoaded.value = false
  const id = props.conversationId
  if (!id || id.startsWith('__')) {
    todoItems.value = []
    todoLoaded.value = true
    return
  }
  try {
    todoItems.value = await invoke<TodoItem[]>('get_todo_tree', { conversationId: id })
  } catch (e) {
    console.warn('get_todo_tree failed', e)
    todoItems.value = []
  }
  todoLoaded.value = true
}

/** 保存 todoTree（整体替换）到后端，成功后刷新本地 */
async function saveTodos() {
  const id = props.conversationId
  if (!id || id.startsWith('__')) return
  try {
    todoItems.value = await invoke<TodoItem[]>('save_todo_tree', {
      conversationId: id,
      todos: todoItems.value,
    })
  } catch (e) {
    toast({ content: `保存任务清单失败：${e}`, type: 'error' })
  }
}

// --- 编辑操作 ---
function newDraft(content = '', status: TodoStatus = 'pending', parentId?: string | null): TodoItem {
  return {
    id: '',
    content,
    priority: 'medium',
    status,
    parent_id: parentId ?? null,
  }
}

function beginAddRoot() {
  editing.value['add-root'] = newDraft()
}
  function cancelEdit(key: string) {
    delete editing.value[key]
  }
/** 确认新增：分配 id → 加入列表 → 保存 */
function confirmAdd(key: string) {
  const draft = editing.value[key]
  if (!draft || !draft.content.trim()) return
  const item: TodoItem = {
    ...draft,
    id: draft.id || genId(),
    content: draft.content.trim(),
  }
  todoItems.value.push(item)
  delete editing.value[key]
  void saveTodos()
}
  function clearTodos() {
    todoItems.value = []
    void saveTodos()
  }

let idCounter = 0
function genId(): string {
  idCounter += 1
  return `u${Date.now().toString(36)}${idCounter.toString(36)}`
}

const todoStats = computed(() => {
  let total = 0
  let inProgress = 0
  let pending = 0
  let completed = 0
  for (const t of todoItems.value) {
    total++
    if (t.status === 'in_progress') inProgress++
    else if (t.status === 'pending') pending++
    else completed++
  }
  return { total, inProgress, pending, completed }
})

// ===================== 上下文窗口 =====================
const usedPct = computed(() => {
  const max = props.contextMaxTokens || 1
  return Math.min(100, Math.round(((props.contextUsedTokens || 0) / max) * 100))
})
const contextStatus = computed<'ok' | 'warning' | 'over'>(() => {
  if (usedPct.value >= 100) return 'over'
  if (usedPct.value >= (compressSettings.value?.threshold_percent ?? 80)) return 'warning'
  return 'ok'
})
const statusLabel = computed(() => {
  if (contextStatus.value === 'over') return '超出'
  if (contextStatus.value === 'warning') return '即将压缩'
  return '充足'
})
const distanceToCompress = computed(() => {
  const threshold = compressSettings.value?.threshold_percent ?? 80
  return Math.max(0, threshold - usedPct.value)
})
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
}

// ===================== 压缩设置（f6） =====================
const compressSettings = ref<CompressionSettings>({
  threshold_percent: 80,
  auto_compress: true,
  compress_tool_calls: true,
  compress_sentences: true,
})

async function loadConfig() {
  try {
    const cfg = await invoke<AgentConfig>('get_config')
    if (cfg.compression_settings) compressSettings.value = cfg.compression_settings
  } catch {
    /* 默认值 */
  }
}
function saveSettings() {
  void (async () => {
    try {
      const cfg = await invoke<AgentConfig>('get_config')
      cfg.compression_settings = { ...compressSettings.value }
      await invoke('set_config', { config: cfg })
      toast({ content: '压缩设置已保存', type: 'success' })
    } catch (e) {
      toast({ content: `保存压缩设置失败：${e}`, type: 'error' })
    }
  })()
}

// ===================== 压缩状态 & 决策列表 =====================
const compressState = ref<CompressionState | null>(null)
const compressing = ref(false)

async function loadCompression() {
  const id = props.conversationId
  if (!id || id.startsWith('__')) {
    compressState.value = null
    return
  }
  try {
    compressState.value = await invoke<CompressionState | null>('get_compression_state', {
      conversationId: id,
    })
  } catch {
    compressState.value = null
  }
}

async function runCompress() {
  const id = props.conversationId
  if (!id || compressing.value) return
  compressing.value = true
  try {
    await invoke('compress_messages_stream', { conversationId: id })
    await loadCompression()
    toast({ content: '压缩完成', type: 'success' })
  } catch (e) {
    toast({ content: `压缩失败：${e}`, type: 'error' })
  } finally {
    compressing.value = false
  }
}
async function clearCompression() {
  const id = props.conversationId
  if (!id) return
  try {
    await invoke('clear_compression_state', { conversationId: id })
    compressState.value = null
    toast({ content: '已清除压缩状态', type: 'success' })
  } catch (e) {
    toast({ content: `清除失败：${e}`, type: 'error' })
  }
}

// 决策计数
  const actionStats = computed(() => {
    const actions = compressState.value?.actions ?? []
    let keep = 0
    let hide = 0
    let replace = 0
    for (const a of actions) {
      if (a.method === 'keep') keep++
      else if (a.method === 'hide') hide++
      else replace++
    }
    return { keep, hide, replace, total: actions.length }
  })

function fmtYuan(n: number): string {
  if (!Number.isFinite(n)) return '—'
  if (n === 0) return '0'
  const abs = Math.abs(n)
  let digits: number
  if (abs >= 1) digits = 2
  else if (abs >= 0.01) digits = 4
  else if (abs >= 0.0001) digits = 6
  else digits = 8
  return n.toFixed(digits).replace(/\.?0+$/, '')
}

// ===================== 用量指标 / 用量分析 =====================
const usage = computed(() => {
  let cacheHit = 0
  let cacheMiss = 0
  let output = 0
  let requests = 0
  let subTokens = 0
  for (const m of props.messages ?? []) {
    if (m.role !== 'assistant') continue
    if (m.usage) {
      cacheHit += m.usage.cache_hit_tokens
      cacheMiss += m.usage.cache_miss_tokens
      output += m.usage.output_tokens
      requests += 1
    }
    // 子代理 token 估算：子代理文本长度 / 4
    for (const sa of m.subAgents ?? []) {
      subTokens += Math.ceil((sa.text?.length ?? 0) / 4)
    }
  }
  const mainTokens = cacheHit + cacheMiss + output
  const p = props.pricing
  const priced = !!p
  const hitCost = priced ? (cacheHit * p!.cache_hit_per_m) / 1e6 : 0
  const missCost = priced ? (cacheMiss * p!.cache_miss_per_m) / 1e6 : 0
  const outputCost = priced ? (output * p!.output_per_m) / 1e6 : 0
  const mainCost = hitCost + missCost + outputCost
  // 子代理费用估算：用输出单价（无子代理独立计费数据）
  const subCost = priced ? (subTokens * (p!.output_per_m || 0)) / 1e6 : 0
  return {
    cacheHit,
    cacheMiss,
    output,
    mainTokens,
    totalTokens: mainTokens + subTokens,
    requests,
    hitRate: cacheHit + cacheMiss > 0 ? (cacheHit / (cacheHit + cacheMiss)) * 100 : 0,
    mainCost,
    subCost,
    totalCost: mainCost + subCost,
    subTokens,
    priced,
  }
})

// 会话运行时间（从首条到末条消息）
const runtime = computed(() => {
  const msgs = props.messages ?? []
  if (msgs.length === 0) return '—'
  const first = msgs[0]?.timestamp ?? 0
  const last = msgs[msgs.length - 1]?.timestamp ?? first
  const diff = Math.max(0, last - first)
  const m = Math.floor(diff / 60000)
  const s = Math.floor((diff % 60000) / 1000)
  return `${m}分${String(s).padStart(2, '0')}秒`
})

const sourcePct = computed(() => {
  const total = usage.value.totalTokens || 1
  const main = (usage.value.mainTokens / total) * 100
  const sub = (usage.value.subTokens / total) * 100
  return { main, sub }
})

// 主模型 / 子代理明细（卡片4 的 child-card）
const mainDetail = computed(() => ({
  label: '主模型',
  count: usage.value.requests,
  total: usage.value.mainTokens,
  cacheRate: usage.value.mainTokens > 0 ? (usage.value.cacheHit / usage.value.mainTokens) * 100 : 0,
  cost: usage.value.mainCost,
  input: usage.value.cacheHit + usage.value.cacheMiss,
  output: usage.value.output,
  hit: usage.value.cacheHit,
  miss: usage.value.cacheMiss,
}))
const subDetail = computed(() => ({
  label: '子代理',
  count: (props.messages ?? []).reduce((n, m) => n + (m.subAgents?.length ?? 0), 0),
  total: usage.value.subTokens,
  cacheRate: 0,
  cost: usage.value.subCost,
  input: 0,
  output: usage.value.subTokens,
  hit: 0,
  miss: 0,
}))

// ===================== 事件 & 生命周期 =====================
let unlistens: UnlistenFn[] = []

watch(
  () => props.conversationId,
  () => {
    loadTodos()
    loadCompression()
  },
)

onMounted(async () => {
  loadTodos()
  loadCompression()
  loadConfig()
  unlistens.push(
    await listen<{ conversation_id: string }>('todo-tree-updated', (e) => {
      if (!props.conversationId || e.payload.conversation_id === props.conversationId) {
        loadTodos()
      }
    }),
  )
  // 压缩完成事件 → 刷新压缩状态
  unlistens.push(
    await listen<{ conversation_id: string }>('agent-compress-done', (e) => {
      if (!props.conversationId || e.payload.conversation_id === props.conversationId) {
        loadCompression()
      }
    }),
  )
})

onUnmounted(() => {
  unlistens.forEach((fn) => fn?.())
  unlistens = []
})

function actionLabel(m: CompressionAction): string {
  if (m.method === 'keep') return '保持'
  if (m.method === 'hide') return '隐藏'
  return '替换'
}
</script>

<template>
  <aside class="ctx-panel">
    <!-- 页签头 -->
    <div class="ctx-tabs">
      <button
        type="button"
        class="ctx-tab"
        :class="{ active: activeTab === 'overview' }"
        @click="activeTab = 'overview'"
      >
          <Icon name="view" :size="15" />
        <span>概览</span>
      </button>
      <button
        type="button"
        class="ctx-tab"
        :class="{ active: activeTab === 'compress' }"
        @click="activeTab = 'compress'"
      >
          <Icon name="merge" :size="15" />
        <span>压缩上下文</span>
      </button>
    </div>

    <div class="ctx-body">
      <!-- ==================== 概览 ==================== -->
      <template v-if="activeTab === 'overview'">
        <!-- card1: todoTree -->
        <section class="ctx-card">
          <div class="ctx-card-head">
            <span class="ctx-card-title">todoTree</span>
            <span v-if="todoStats.total > 0" class="ctx-card-badge">
              {{ todoStats.total }} 项 · {{ todoStats.inProgress }} 进行中
            </span>
            <div class="ctx-card-actions">
              <IconButton size="sm" icon="plus" title="添加根任务" @click="beginAddRoot" />
              <IconButton size="sm" icon="delete" title="清空" @click="clearTodos" />
            </div>
          </div>

          <div v-if="!todoLoaded" class="ctx-empty">加载中…</div>
          <div v-else-if="todoTree.length === 0 && !editing['add-root']" class="ctx-empty">
            暂无任务。发给 agent 一个复杂任务，它会用 todo_write 建立任务树；或点击 + 手动添加。
          </div>
          <div v-else class="todo-tree">
            <!-- 根任务添加框 -->
            <div v-if="editing['add-root']" class="todo-add">
              <input
                v-model="editing['add-root'].content"
                class="todo-input"
                placeholder="新任务…"
                @keydown.enter="confirmAdd('add-root')"
                @keydown.esc="cancelEdit('add-root')"
              />
              <button type="button" class="todo-ok" @click="confirmAdd('add-root')">
                <Icon name="check" :size="14" />
              </button>
              <button type="button" class="todo-x" @click="cancelEdit('add-root')">
                <Icon name="close" :size="14" />
              </button>
            </div>

              <!-- 递归渲染树节点（任意深度，TodoTreeNode 递归展开） -->
              <template v-for="node in todoTree" :key="node.id">
                <TodoTreeNode
                  :node="node"
                  :depth="0"
                  :items="todoItems"
                  :editing="editing"
                  :gen-id="genId"
                  :on-changed="saveTodos"
                />
              </template>
          </div>
        </section>

        <!-- card2: 上下文窗口 -->
        <section class="ctx-card">
          <div class="ctx-card-head">
            <span class="ctx-card-title">上下文窗口</span>
            <span class="ctx-status" :class="contextStatus">{{ statusLabel }}</span>
            <span class="ctx-max">{{ fmtTokens(contextMaxTokens || 0) }}</span>
          </div>
          <div class="ctx-bar-row">
            <div class="ctx-bar">
              <div class="ctx-bar-fill" :class="contextStatus" :style="{ width: usedPct + '%' }" />
            </div>
          </div>
          <div class="ctx-bar-meta">
            <span>已用 {{ usedPct }}%</span>
            <span v-if="contextStatus !== 'over'">距离压缩 {{ distanceToCompress }}%</span>
            <span v-else>已超出上限</span>
          </div>
        </section>

        <!-- card3: 用量指标 -->
        <section class="ctx-card">
          <div class="ctx-card-head">
            <span class="ctx-card-title">用量指标</span>
          </div>
          <div class="usage-grid">
            <div class="usage-cell">
              <span class="usage-label">平均命中</span>
              <span class="usage-val">{{ usage.hitRate.toFixed(2) }}%</span>
            </div>
            <div class="usage-cell">
              <span class="usage-label">会话费用</span>
              <span class="usage-val">{{ usage.priced ? `¥${fmtYuan(usage.totalCost)}` : `${fmtTokens(usage.totalTokens)} tokens` }}</span>
            </div>
            <div class="usage-cell">
              <span class="usage-label">运行时间</span>
              <span class="usage-val">{{ runtime }}</span>
            </div>
            <div class="usage-cell">
              <span class="usage-label">请求数</span>
              <span class="usage-val">{{ usage.requests }}</span>
            </div>
            <div class="usage-cell">
              <span class="usage-label">缓存</span>
              <span class="usage-val">{{ fmtTokens(usage.cacheHit) }}</span>
            </div>
            <div class="usage-cell">
              <span class="usage-label">输入未缓存</span>
              <span class="usage-val">{{ fmtTokens(usage.cacheMiss) }}</span>
            </div>
            <div class="usage-cell usage-cell--wide">
              <span class="usage-label">输出</span>
              <span class="usage-val">{{ fmtTokens(usage.output) }}</span>
            </div>
          </div>
        </section>

        <!-- card4: 用量分析 -->
        <section class="ctx-card">
          <div class="ctx-card-head">
            <span class="ctx-card-title">用量分析</span>
          </div>
          <div class="source-label">来源占比（按费用）</div>
          <div class="source-bar">
            <div class="source-main" :style="{ width: sourcePct.main + '%' }">主模型 {{ sourcePct.main.toFixed(0) }}%</div>
            <div class="source-sub" :style="{ width: sourcePct.sub + '%' }">子代理 {{ sourcePct.sub.toFixed(0) }}%</div>
          </div>
          <div class="source-cards">
            <!-- 主模型 child-card -->
            <div class="source-card">
              <div class="source-card-head">{{ mainDetail.label }}</div>
              <div class="source-row"><span>次数</span><b>{{ mainDetail.count }}</b></div>
              <div class="source-row"><span>总计</span><b>{{ fmtTokens(mainDetail.total) }}</b></div>
              <div class="source-row"><span>缓存</span><b>{{ mainDetail.cacheRate.toFixed(2) }}%</b></div>
              <div class="source-row"><span>费用</span><b>{{ usage.priced ? `¥${fmtYuan(mainDetail.cost)}` : '—' }}</b></div>
              <div class="source-detail">
                <div class="source-row"><span>输入</span><b>{{ fmtTokens(mainDetail.input) }}</b></div>
                <div class="source-row"><span>输出</span><b>{{ fmtTokens(mainDetail.output) }}</b></div>
                <div class="source-row"><span>命中</span><b>{{ fmtTokens(mainDetail.hit) }}</b></div>
                <div class="source-row"><span>未命中</span><b>{{ fmtTokens(mainDetail.miss) }}</b></div>
              </div>
            </div>
            <!-- 子代理 child-card -->
            <div class="source-card">
              <div class="source-card-head">{{ subDetail.label }}</div>
              <div class="source-row"><span>次数</span><b>{{ subDetail.count }}</b></div>
              <div class="source-row"><span>总计</span><b>{{ fmtTokens(subDetail.total) }}</b></div>
              <div class="source-row"><span>缓存</span><b>—</b></div>
              <div class="source-row"><span>费用</span><b>{{ usage.priced ? `¥${fmtYuan(subDetail.cost)}` : '—' }}</b></div>
              <div class="source-detail">
                <div class="source-row"><span>输出</span><b>{{ fmtTokens(subDetail.output) }}</b></div>
              </div>
            </div>
          </div>
        </section>
      </template>

      <!-- ==================== 压缩上下文 ==================== -->
      <template v-else>
        <!-- 压缩机制设置（f6） -->
        <section class="ctx-card">
          <div class="ctx-card-head">
            <span class="ctx-card-title">压缩机制</span>
          </div>
          <div class="setting-row">
            <div class="setting-label">自动压缩阈值</div>
            <div class="setting-slider">
              <Slider
                v-model="compressSettings.threshold_percent"
                :min="1"
                :max="100"
                :step="1"
                size="sm"
                show-value
                @change="saveSettings"
              />
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-label">达到阈值自动压缩</div>
            <Switch v-model="compressSettings.auto_compress" @change="saveSettings" />
          </div>
          <div class="setting-row">
            <div class="setting-label">工具调用 / 返回压缩</div>
            <Switch v-model="compressSettings.compress_tool_calls" @change="saveSettings" />
          </div>
          <div class="setting-row">
            <div class="setting-label">逐句对话压缩</div>
            <Switch v-model="compressSettings.compress_sentences" @change="saveSettings" />
          </div>
        </section>

        <!-- 压缩操作 -->
        <section class="ctx-card">
          <div class="ctx-card-head">
            <span class="ctx-card-title">压缩操作</span>
            <span v-if="compressState" class="ctx-card-badge">
              上次 {{ new Date(compressState.updated_at).toLocaleTimeString() }}
            </span>
          </div>
          <div class="compress-actions">
            <Button size="sm" :loading="compressing" @click="runCompress">立即压缩</Button>
              <Button v-if="compressState" size="sm" variant="text" @click="clearCompression">
              清除压缩状态
            </Button>
          </div>
          <div v-if="actionStats.total > 0" class="compress-stats">
            <span class="cs cs-keep">保持 {{ actionStats.keep }}</span>
            <span class="cs cs-hide">隐藏 {{ actionStats.hide }}</span>
            <span class="cs cs-replace">替换 {{ actionStats.replace }}</span>
          </div>
        </section>

        <!-- 决策列表 -->
        <section class="ctx-card">
          <div class="ctx-card-head">
            <span class="ctx-card-title">压缩决策</span>
          </div>
          <div v-if="!compressState || compressState.actions.length === 0" class="ctx-empty">
            尚未压缩。压缩只影响后续发给 LLM 的 prompt，界面仍显示原始消息。
          </div>
          <div v-else class="action-list">
            <div
              v-for="(a, i) in compressState.actions"
              :key="i"
              class="action-item"
              :class="a.method"
            >
              <span class="action-method">{{ actionLabel(a) }}</span>
              <span class="action-ids">{{ a.message_ids.length }} 条</span>
              <span class="action-reason" :title="a.reason">{{ a.reason }}</span>
            </div>
          </div>
        </section>
      </template>
    </div>
  </aside>
</template>

<style scoped>
.ctx-panel {
  display: flex;
  flex-direction: column;
  width: 300px;
  min-width: 300px;
  height: 100%;
  border-left: 1px solid var(--border);
  background: var(--bg);
  overflow: hidden;
}

.ctx-tabs {
  display: flex;
  gap: 4px;
  padding: 8px;
  border-bottom: 1px solid var(--border);
  background: var(--card);
  flex-shrink: 0;
}

.ctx-tab {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 6px 10px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  font-size: var(--fs-sm);
  cursor: pointer;
  transition: all 0.15s ease;
}
.ctx-tab:hover {
  background: var(--hover);
  color: var(--text);
}
.ctx-tab.active {
  background: color-mix(in srgb, var(--primary) 14%, var(--card));
  color: var(--primary);
  font-weight: 600;
}

.ctx-body {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.ctx-card {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 10px 12px;
  flex-shrink: 0;
}
.ctx-card-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.ctx-card-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}
.ctx-card-badge {
  font-size: var(--fs-xs);
  color: var(--muted);
  margin-left: auto;
}
.ctx-card-actions {
  margin-left: auto;
  display: flex;
  gap: 2px;
}
.ctx-empty {
  font-size: var(--fs-xs);
  color: var(--muted);
  padding: 8px 0;
  line-height: 1.6;
}

/* ---------- todoTree ---------- */
.todo-tree {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.todo-node {
  display: flex;
  flex-direction: column;
}
.todo-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 2px;
  border-radius: var(--radius-xs);
}
.todo-row:hover {
  background: var(--hover);
}
.todo-row--deep {
  padding-left: 40px;
}
.todo-status {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  flex-shrink: 0;
}
.todo-status.completed {
  color: var(--success);
}
.todo-status.in_progress {
  color: var(--primary);
}
.todo-content {
  flex: 1;
  font-size: var(--fs-sm);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}
.todo-content.done {
  color: var(--muted);
  text-decoration: line-through;
}
.todo-priority {
  font-size: var(--fs-xs);
  padding: 1px 6px;
  border-radius: var(--radius-full);
  cursor: pointer;
  flex-shrink: 0;
}
.todo-priority.high {
  background: color-mix(in srgb, #f5222d 14%, var(--card));
  color: #f5222d;
}
.todo-priority.medium {
  background: color-mix(in srgb, #fa8c16 14%, var(--card));
  color: #fa8c16;
}
.todo-priority.low {
  background: color-mix(in srgb, #52c41a 14%, var(--card));
  color: #52c41a;
}
.todo-ops {
  display: none;
  gap: 2px;
  flex-shrink: 0;
}
.todo-row:hover .todo-ops {
  display: inline-flex;
}
.todo-children {
  padding-left: 22px;
  border-left: 1px dashed var(--border);
  margin-left: 9px;
}
.todo-children--deep {
  padding-left: 10px;
  margin-left: 0;
  border-left: none;
}
.todo-add {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 2px;
}
.todo-add--edit {
  padding-left: 26px;
}
.todo-add--child {
  padding-left: 40px;
}
.todo-input {
  flex: 1;
  min-width: 0;
  height: 24px;
  padding: 0 8px;
  font-size: var(--fs-sm);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  outline: none;
}
.todo-input:focus {
  border-color: var(--primary);
}
.todo-ok,
.todo-x {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  cursor: pointer;
  color: var(--muted);
}
.todo-ok:hover {
  color: var(--success);
  background: var(--hover);
}
.todo-x:hover {
  color: var(--danger);
  background: var(--hover);
}

/* ---------- 上下文窗口 ---------- */
.ctx-status {
  font-size: var(--fs-xs);
  padding: 1px 8px;
  border-radius: var(--radius-full);
}
.ctx-status.ok {
  background: color-mix(in srgb, var(--success) 14%, var(--card));
  color: var(--success);
}
.ctx-status.warning {
  background: color-mix(in srgb, #fa8c16 14%, var(--card));
  color: #fa8c16;
}
.ctx-status.over {
  background: color-mix(in srgb, #f5222d 14%, var(--card));
  color: #f5222d;
}
.ctx-max {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--muted);
}
.ctx-bar-row {
  margin: 6px 0 4px;
}
.ctx-bar {
  position: relative;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--border);
  overflow: hidden;
}
.ctx-bar-fill {
  height: 100%;
  border-radius: var(--radius-full);
  background: var(--primary);
  transition: width 0.3s ease;
}
.ctx-bar-fill.warning {
  background: #fa8c16;
}
.ctx-bar-fill.over {
  background: #f5222d;
}
.ctx-bar-meta {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* ---------- 用量指标 ---------- */
.usage-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
}
.usage-cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 8px;
  background: var(--bg);
  border-radius: var(--radius-sm);
}
.usage-cell--wide {
  grid-column: 1 / -1;
}
.usage-label {
  font-size: var(--fs-xs);
  color: var(--muted);
}
.usage-val {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
}

/* ---------- 用量分析 ---------- */
.source-label {
  font-size: var(--fs-xs);
  color: var(--muted);
  margin-bottom: 4px;
}
.source-bar {
  display: flex;
  height: 18px;
  border-radius: var(--radius-sm);
  overflow: hidden;
  font-size: var(--fs-xs);
  color: #fff;
  margin-bottom: 8px;
}
.source-main {
  background: var(--primary);
  display: flex;
  align-items: center;
  justify-content: center;
  white-space: nowrap;
  overflow: hidden;
}
.source-sub {
  background: color-mix(in srgb, var(--primary) 45%, var(--muted));
  display: flex;
  align-items: center;
  justify-content: center;
  white-space: nowrap;
  overflow: hidden;
}
.source-cards {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
}
.source-card {
  padding: 6px 8px;
  background: var(--bg);
  border-radius: var(--radius-sm);
}
.source-card-head {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text);
  margin-bottom: 4px;
}
.source-row {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-xs);
  color: var(--muted);
  padding: 1px 0;
}
.source-row b {
  color: var(--text);
  font-weight: 600;
}
.source-detail {
  margin-top: 4px;
  border-top: 1px dashed var(--border);
  padding-top: 2px;
}

/* ---------- 压缩设置 ---------- */
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 0;
}
.setting-label {
  font-size: var(--fs-sm);
  color: var(--text);
}
.setting-slider {
  width: 130px;
}
.compress-actions {
  display: flex;
  gap: 6px;
}
.compress-stats {
  display: flex;
  gap: 8px;
  margin-top: 8px;
  font-size: var(--fs-xs);
}
.cs-keep {
  color: var(--success);
}
.cs-hide {
  color: var(--muted);
}
.cs-replace {
  color: #fa8c16;
}

/* ---------- 决策列表 ---------- */
.action-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.action-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 6px;
  border-radius: var(--radius-sm);
  background: var(--bg);
  font-size: var(--fs-xs);
}
.action-method {
  padding: 1px 6px;
  border-radius: var(--radius-full);
  font-weight: 600;
  flex-shrink: 0;
}
.action-item.keep .action-method {
  background: color-mix(in srgb, var(--success) 14%, var(--card));
  color: var(--success);
}
.action-item.hide .action-method {
  background: color-mix(in srgb, var(--muted) 18%, var(--card));
  color: var(--muted);
}
.action-item.replace .action-method {
  background: color-mix(in srgb, #fa8c16 14%, var(--card));
  color: #fa8c16;
}
.action-ids {
  color: var(--muted);
  flex-shrink: 0;
}
.action-reason {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--muted);
}
</style>
