<script setup lang="ts">
/**
 * AgentFlowView 智能体流程（ComfyUI 风格可视化节点编辑器）
 *
 * 两种模式：
 * - 列表模式：浏览 / 新建 / 删除已保存的流程
 * - 编辑模式：画布上拖拽节点、连线、编辑参数，保存后持久化到后端
 *
 * 通过 Tauri invoke 调用后端命令：
 *   list_agent_flows / get_agent_flow / save_agent_flow / delete_agent_flow
 */
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { animate } from 'animejs'
import { Button, IconButton, Icon, Dialog, Chips, useToast } from '../basic'
import type { AgentFlow, FlowNode, FlowDataType } from '../../types'

const { toast } = useToast()

// ================= 类型 / 常量 =================
/** 端口类型 → 颜色（与后端 FlowDataType::color 一致） */
const TYPE_COLORS: Record<FlowDataType, string> = {
  text: '#4a7eff',
  file: '#f2994a',
  image: '#e5484d',
  audio: '#8e4ec6',
  object: '#30a46c',
  number: '#6e56cf',
}

const TYPE_LABEL: Record<FlowDataType, string> = {
  text: '文本',
  file: '文件',
  image: '图片',
  audio: '音频',
  object: '对象',
  number: '数字',
}

/** 画布舞台尺寸（节点/连线共用同一坐标系） */
const STAGE_W = 2000
const STAGE_H = 2000
const NODE_W = 200
const HEADER_H = 40
const PORT_Y = HEADER_H + 18 // 端口中心相对节点顶部的 y

/** 预置节点库模板 */
interface NodeTemplate {
  type: string
  label: string
  icon: string
  input_type: FlowDataType
  output_type: FlowDataType
  params: Record<string, unknown>
}

const NODE_LIBRARY: NodeTemplate[] = [
  { type: 'input', label: '输入', icon: 'arrow-right', input_type: 'object', output_type: 'text', params: { prompt: '' } },
  { type: 'output', label: '输出', icon: 'send', input_type: 'text', output_type: 'object', params: {} },
  { type: 'text', label: '文本处理', icon: 'message', input_type: 'text', output_type: 'text', params: { operation: 'concat', separator: '\n' } },
  { type: 'agent', label: '调用智能体', icon: 'robot', input_type: 'text', output_type: 'text', params: { skill_id: '', instruction: '' } },
  { type: 'file', label: '文件处理', icon: 'file', input_type: 'file', output_type: 'file', params: { path: '' } },
  { type: 'image', label: '图片处理', icon: 'image', input_type: 'image', output_type: 'image', params: { width: 512, height: 512 } },
  { type: 'audio', label: '音频处理', icon: 'mic', input_type: 'audio', output_type: 'audio', params: {} },
  { type: 'number', label: '数值处理', icon: 'bolt', input_type: 'number', output_type: 'number', params: { factor: 1 } },
]

// ================= 模式 & 数据 =================
const mode = ref<'list' | 'edit'>('list')
const flows = ref<AgentFlow[]>([])
const editing = ref<AgentFlow | null>(null)
const loading = ref(false)
const saving = ref(false)

const editingNodes = computed<FlowNode[]>(() => editing.value?.nodes ?? [])
const editingEdges = computed(() => editing.value?.edges ?? [])

function nodeById(id: string): FlowNode | undefined {
  return editingNodes.value.find((n) => n.id === id)
}

// ================= 列表 =================
async function refresh() {
  loading.value = true
  try {
    flows.value = await invoke<AgentFlow[]>('list_agent_flows')
  } catch (e) {
    toast({ content: `加载流程失败：${e}`, type: 'error' })
    flows.value = []
  } finally {
    loading.value = false
  }
}

onMounted(refresh)

function formatTime(ts: number | null | undefined): string {
  if (!ts) return '—'
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

// ---------- 新建流程 ----------
const createDialogOpen = ref(false)
const draftName = ref('')
const draftDesc = ref('')

function openCreate() {
  draftName.value = ''
  draftDesc.value = ''
  createDialogOpen.value = true
}

async function confirmCreate() {
  const name = draftName.value.trim()
  if (!name) {
    toast({ content: '请输入流程名称', type: 'warn' })
    return
  }
  try {
    const flow: AgentFlow = {
      id: '',
      name,
      description: draftDesc.value.trim(),
      nodes: [],
      edges: [],
      created_at: 0,
      updated_at: 0,
    }
    const saved = await invoke<AgentFlow>('save_agent_flow', { flow })
    createDialogOpen.value = false
    editing.value = saved
    mode.value = 'edit'
    await refresh()
  } catch (e) {
    toast({ content: `创建失败：${e}`, type: 'error' })
  }
}

// ---------- 打开编辑 ----------
async function openEdit(flow: AgentFlow) {
  try {
    const saved = await invoke<AgentFlow | null>('get_agent_flow', { id: flow.id })
    if (!saved) {
      toast({ content: '流程不存在或已被删除', type: 'warn' })
      await refresh()
      return
    }
    editing.value = saved
    mode.value = 'edit'
  } catch (e) {
    toast({ content: `打开失败：${e}`, type: 'error' })
  }
}

// ---------- 删除 ----------
const deleteDialogOpen = ref(false)
const deleteTarget = ref<AgentFlow | null>(null)

function askDelete(flow: AgentFlow) {
  deleteTarget.value = flow
  deleteDialogOpen.value = true
}

async function confirmDelete() {
  const t = deleteTarget.value
  if (!t) return
  try {
    await invoke('delete_agent_flow', { id: t.id })
    toast({ content: `已删除流程「${t.name}」`, type: 'success' })
    deleteTarget.value = null
    // 若正在编辑的就是被删对象，回到列表
    if (editing.value?.id === t.id) {
      editing.value = null
      mode.value = 'list'
    }
    await refresh()
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
    deleteTarget.value = null
  }
}

// ---------- 返回列表 ----------
function backToList() {
  mode.value = 'list'
  editing.value = null
  armedType.value = null
  endLink()
}

// ---------- 保存 ----------
async function saveFlow() {
  const f = editing.value
  if (!f) return
  saving.value = true
  try {
    const saved = await invoke<AgentFlow>('save_agent_flow', { flow: f })
    editing.value = saved
    toast({ content: `已保存流程「${saved.name}」`, type: 'success' })
    mode.value = 'list'
    backToList()
    await refresh()
  } catch (e) {
    toast({ content: `保存失败：${e}`, type: 'error' })
  } finally {
    saving.value = false
  }
}

// ================= 节点 id 生成 =================
let uidCounter = 0
function uid(prefix: string): string {
  uidCounter += 1
  return `${prefix}_${Date.now().toString(36)}_${uidCounter.toString(36)}`
}

// ================= 添加节点 =================
const armedType = ref<NodeTemplate | null>(null)

function toggleArm(t: NodeTemplate, on: boolean) {
  armedType.value = on ? t : null
}

function addNodeAt(x: number, y: number, t: NodeTemplate) {
  const f = editing.value
  if (!f) return
  f.nodes.push({
    id: uid('n'),
    node_type: t.type,
    label: t.label,
    x,
    y,
    params: { ...t.params },
    input_type: t.input_type,
    output_type: t.output_type,
  })
}

// 从节点库快速添加（级联排列，便于连续放置）
let quickCount = 0
function quickAdd() {
  const t = armedType.value
  if (!t) {
    toast({ content: '请先在节点库中选择节点类型', type: 'warn' })
    return
  }
  const x = 80 + (quickCount % 6) * 40
  const y = 80 + (quickCount % 8) * 60
  quickCount += 1
  addNodeAt(x, y, t)
}

// ================= 删除节点 =================
function deleteNode(id: string) {
  const f = editing.value
  if (!f) return
  f.nodes = f.nodes.filter((n) => n.id !== id)
  f.edges = f.edges.filter((e) => e.from !== id && e.to !== id)
  if (linkFrom.value === id) endLink()
}

// ================= 连线 =================
const linking = ref(false)
const linkFrom = ref<string | null>(null)
const pointer = ref({ x: 0, y: 0 })
// 抑制端口/拖拽产生的 click 冒泡，避免误开参数弹窗
const suppressClick = ref(false)

function startLink(node: FlowNode, ev: PointerEvent) {
  ev.stopPropagation()
  suppressClick.value = true
  if (linking.value) endLink()
  linking.value = true
  linkFrom.value = node.id
  window.addEventListener('pointermove', onLinkMove)
  window.addEventListener('pointerup', onLinkUp)
}

function finishLink(toNode: FlowNode, ev: PointerEvent) {
  ev.stopPropagation()
  suppressClick.value = true
  if (linking.value && linkFrom.value && linkFrom.value !== toNode.id) {
    addEdge(linkFrom.value, toNode.id)
  }
  endLink()
}

function addEdge(from: string, to: string) {
  const f = editing.value
  if (!f) return
  const dup = f.edges.some((e) => e.from === from && e.to === to)
  if (dup) return
  f.edges.push({ id: uid('e'), from, to })
}

function deleteEdge(id: string, ev: Event) {
  ev.stopPropagation()
  const f = editing.value
  if (!f) return
  f.edges = f.edges.filter((e) => e.id !== id)
}

function onLinkMove(e: PointerEvent) {
  const pt = toStage(e)
  if (pt) pointer.value = pt
}

function onLinkUp() {
  endLink()
}

function endLink() {
  linking.value = false
  linkFrom.value = null
  window.removeEventListener('pointermove', onLinkMove)
  window.removeEventListener('pointerup', onLinkUp)
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (linking.value) endLink()
    if (armedType.value) armedType.value = null
  }
}

// 坐标换算：相对于舞台（stage 左上角）
function toStage(e: PointerEvent): { x: number; y: number } | null {
  const stage = stageEl.value
  if (!stage) return null
  const rect = stage.getBoundingClientRect()
  return { x: e.clientX - rect.left, y: e.clientY - rect.top }
}

const stageEl = ref<HTMLElement | null>(null)

// 舞台点击：放置节点 / 取消连线
function onStageDown(e: PointerEvent) {
  const target = e.target as HTMLElement
  if (target.closest('.flow-node')) return // 命中节点，交给节点自身处理
  if (linking.value) {
    endLink()
    return
  }
  if (armedType.value) {
    const pt = toStage(e)
    if (pt) addNodeAt(pt.x, pt.y, armedType.value)
  }
}

// ================= 拖拽节点 =================
const dragId = ref<string | null>(null)
const dragOffset = ref({ x: 0, y: 0 })

function startDrag(node: FlowNode, ev: PointerEvent) {
  ev.stopPropagation()
  if (armedType.value) armedType.value = null
  const pt = toStage(ev)
  if (!pt) return
  // 给即将拖动的节点一个轻微抬升反馈
  const el = ev.currentTarget as HTMLElement
  const nodeEl = el.closest('.flow-node') as HTMLElement | null
  if (nodeEl) {
    animate(nodeEl, { scale: [1, 1.02], duration: 140, ease: 'out(3)' })
  }
  dragId.value = node.id
  dragOffset.value = { x: pt.x - node.x, y: pt.y - node.y }
  window.addEventListener('pointermove', onDragMove)
  window.addEventListener('pointerup', onDragUp)
}

function onDragMove(e: PointerEvent) {
  const id = dragId.value
  if (!id) return
  const node = nodeById(id)
  const pt = toStage(e)
  if (!node || !pt) return
  node.x = clamp(pt.x - dragOffset.value.x, -NODE_W + 40, STAGE_W - 40)
  node.y = clamp(pt.y - dragOffset.value.y, -HEADER_H, STAGE_H - 40)
}

function onDragUp(e: PointerEvent) {
  const id = dragId.value
  dragId.value = null
  suppressClick.value = true
  window.removeEventListener('pointermove', onDragMove)
  window.removeEventListener('pointerup', onDragUp)
  // 落点轻微弹性回弹
  const node = id ? nodeById(id) : undefined
  const target = e.target as HTMLElement
  const nodeEl = (node ? target.closest('.flow-node') : null) as HTMLElement | null
  if (nodeEl) {
    animate(nodeEl, {
      scale: [1.02, 1],
      duration: 220,
      ease: 'out(3)',
      onComplete: () => {
        nodeEl.style.transform = ''
      },
    })
  }
}

function clamp(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, v))
}

// ================= 参数编辑 =================
const paramDialogOpen = ref(false)
const paramNodeId = ref<string | null>(null)
const paramLabel = ref('')
const paramJson = ref('')

function openParams(node: FlowNode) {
  paramNodeId.value = node.id
  paramLabel.value = node.label
  paramJson.value = JSON.stringify(node.params ?? {}, null, 2)
  paramDialogOpen.value = true
}

function onNodeClick(node: FlowNode) {
  if (suppressClick.value) {
    suppressClick.value = false
    return
  }
  if (linking.value) {
    endLink()
    return
  }
  openParams(node)
}

function saveParams() {
  const nodeId = paramNodeId.value
  if (!nodeId) return
  const node = nodeById(nodeId)
  if (!node) return
  const label = paramLabel.value.trim()
  if (label) node.label = label
  try {
    node.params = JSON.parse(paramJson.value)
  } catch {
    toast({ content: '参数不是合法的 JSON', type: 'error' })
    return
  }
  paramDialogOpen.value = false
}

// ================= 渲染计算 =================
interface EdgeLine {
  id: string
  d: string
  color: string
  from: FlowNode
  to: FlowNode
  x1: number
  y1: number
  x2: number
  y2: number
  mx: number
  my: number
}

const edgeLines = computed<EdgeLine[]>(() =>
  editingEdges.value
    .map((e) => {
      const from = nodeById(e.from)
      const to = nodeById(e.to)
      if (!from || !to) return null
      const x1 = from.x + NODE_W
      const y1 = from.y + PORT_Y
      const x2 = to.x
      const y2 = to.y + PORT_Y
      const dx = Math.max((x2 - x1) / 2, 48)
      const d = `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`
      return {
        id: e.id,
        d,
        color: TYPE_COLORS[from.output_type],
        from,
        to,
        x1,
        y1,
        x2,
        y2,
        mx: (x1 + x2) / 2,
        my: (y1 + y2) / 2,
      }
    })
    .filter((l): l is EdgeLine => l !== null),
)

const tempLine = computed(() => {
  if (!linking.value || !linkFrom.value) return null
  const from = nodeById(linkFrom.value)
  if (!from) return null
  const x1 = from.x + NODE_W
  const y1 = from.y + PORT_Y
  const x2 = pointer.value.x
  const y2 = pointer.value.y
  const dx = Math.max((x2 - x1) / 2, 48)
  return {
    d: `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`,
    color: TYPE_COLORS[from.output_type],
  }
})

onMounted(() => {
  if (typeof window !== 'undefined') window.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
  if (typeof window !== 'undefined') {
    window.removeEventListener('keydown', onKeydown)
    window.removeEventListener('pointermove', onLinkMove)
    window.removeEventListener('pointerup', onLinkUp)
    window.removeEventListener('pointermove', onDragMove)
    window.removeEventListener('pointerup', onDragUp)
  }
})

function colorOf(t: FlowDataType): string {
  return TYPE_COLORS[t]
}
</script>

<template>
  <div class="flow-view">
    <!-- ================= 列表模式 ================= -->
    <div v-if="mode === 'list'" class="list-pane">
      <header class="pane-hero">
        <div class="hero-mark"><Icon name="puzzle" :size="28" /></div>
        <div class="hero-text">
          <h2 class="hero-title">智能体流程</h2>
          <p class="hero-sub">ComfyUI 风格的可视化节点编排，拖拽节点、连线、保存后即可复用</p>
        </div>
      </header>

      <section class="section">
        <div class="section-head">
          <span class="section-title">流程列表</span>
          <span v-if="flows.length" class="count-badge">{{ flows.length }}</span>
        </div>

        <div v-if="!flows.length && !loading" class="empty-state">
          <div class="empty-illust"><Icon name="puzzle" :size="48" /></div>
          <p class="empty-text">还没有流程</p>
          <p class="empty-hint">点击下方按钮，创建你的第一个可视化流程</p>
        </div>

        <div v-else class="flow-list">
          <div v-for="f in flows" :key="f.id" class="flow-card">
            <div class="flow-card-main">
              <div class="flow-top">
                <span class="flow-name">{{ f.name }}</span>
                <span class="flow-desc">{{ f.description || '无描述' }}</span>
              </div>
              <div class="flow-meta">
                <span class="meta-item"><Icon name="puzzle" :size="14" /> {{ f.nodes.length }} 节点</span>
                <span class="meta-item"><Icon name="merge" :size="14" /> {{ f.edges.length }} 连线</span>
                <span class="meta-item meta-time">更新于 {{ formatTime(f.updated_at) }}</span>
              </div>
            </div>
            <div class="flow-card-actions">
              <Button variant="normal" size="sm" @click="openEdit(f)">编辑</Button>
              <IconButton size="sm" title="删除" @click="askDelete(f)"><Icon name="delete" :size="18" /></IconButton>
            </div>
          </div>
        </div>
      </section>

      <div class="list-footer">
        <Button variant="primary" block @click="openCreate">
          <template #icon><Icon name="plus" :size="18" /></template>
          新建流程
        </Button>
      </div>
    </div>

    <!-- ================= 编辑模式 ================= -->
    <div v-else-if="editing" class="edit-pane">
      <!-- 工具栏 -->
      <div class="toolbar">
        <IconButton size="md" title="返回列表" @click="backToList">
          <Icon name="arrow-left" :size="20" />
        </IconButton>
        <input
          v-model="editing.name"
          class="name-input"
          type="text"
          placeholder="流程名称"
          aria-label="流程名称"
        />
        <div class="toolbar-spacer"></div>
        <Button variant="primary" size="sm" :loading="saving" @click="saveFlow">
          <template #icon><Icon name="check" :size="16" /></template>
          保存
        </Button>
        <IconButton size="sm" title="删除此流程" @click="askDelete(editing)">
          <Icon name="delete" :size="18" />
        </IconButton>
      </div>

      <!-- 节点库 -->
      <div class="lib-bar">
        <span class="lib-label">节点库</span>
        <div class="lib-chips">
          <Chips
            v-for="lib in NODE_LIBRARY"
            :key="lib.type"
            :label="lib.label"
            :size="'sm'"
            :selected="armedType?.type === lib.type"
            @update:selected="(v: boolean) => toggleArm(lib, v)"
          >
            <template #icon><Icon :name="lib.icon" :size="14" /></template>
          </Chips>
        </div>
        <Button variant="normal" size="sm" class="quick-add" @click="quickAdd">
          <template #icon><Icon name="plus" :size="16" /></template>
          添加
        </Button>
      </div>

      <div v-if="armedType" class="arm-hint">
        <Icon name="move" :size="14" />
        已选择「{{ armedType.label }}」，点击画布放置节点（Esc 取消）
        <button type="button" class="arm-cancel" @click="armedType = null">取消</button>
      </div>

      <!-- 画布 -->
      <div class="canvas-wrap">
        <div
          ref="stageEl"
          class="stage"
          :style="{ width: STAGE_W + 'px', height: STAGE_H + 'px' }"
          @pointerdown="onStageDown"
        >
          <!-- 连线层 -->
          <svg class="edges" :width="STAGE_W" :height="STAGE_H">
            <g v-for="l in edgeLines" :key="l.id">
              <path :d="l.d" class="edge-path" :stroke="l.color" />
              <circle
                :cx="l.mx"
                :cy="l.my"
                r="7"
                class="edge-hit"
                :title="`删除连线 ${l.from.label} → ${l.to.label}`"
                @pointerdown="(e: PointerEvent) => deleteEdge(l.id, e)"
              />
            </g>
            <path
              v-if="tempLine"
              :d="tempLine.d"
              class="edge-temp"
              :stroke="tempLine.color"
            />
          </svg>

          <!-- 节点层 -->
          <TransitionGroup name="node" tag="div" class="node-layer">
            <div
              v-for="n in editingNodes"
              :key="n.id"
              class="flow-node"
              :style="{ left: n.x + 'px', top: n.y + 'px' }"
              @click="onNodeClick(n)"
            >
              <div class="node-head" @pointerdown.stop="(e: PointerEvent) => startDrag(n, e)">
                <span class="node-title">{{ n.label }}</span>
                <button
                  type="button"
                  class="node-del"
                  title="删除节点"
                  @pointerdown.stop
                  @click.stop="deleteNode(n.id)"
                ><Icon name="close" :size="14" /></button>
              </div>

              <div class="node-ports">
                <div class="node-port node-port--in">
                  <span class="port-dot" @pointerdown.stop="(e: PointerEvent) => finishLink(n, e)"></span>
                  <span class="port-label">{{ TYPE_LABEL[n.input_type] }}</span>
                </div>
                <div class="node-port node-port--out">
                  <span class="port-label">{{ TYPE_LABEL[n.output_type] }}</span>
                  <span
                    class="port-dot"
                    :style="{ background: colorOf(n.output_type), borderColor: colorOf(n.output_type) }"
                    @pointerdown.stop="(e: PointerEvent) => startLink(n, e)"
                  ></span>
                </div>
              </div>

              <div class="node-foot">
                <span class="foot-dot" :style="{ background: colorOf(n.output_type) }"></span>
                <span>参数 ×{{ Object.keys(n.params ?? {}).length }}</span>
              </div>
            </div>
          </TransitionGroup>
        </div>
      </div>
    </div>

    <!-- ================= 新建流程 Dialog ================= -->
    <Dialog
      v-model:visible="createDialogOpen"
      title="新建智能体流程"
      confirm-text="创建"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmCreate"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">流程名称</label>
          <input v-model="draftName" type="text" class="field-input" placeholder="例如：每日自动报告" />
        </div>
        <div class="field">
          <label class="field-label">描述（可选）</label>
          <input v-model="draftDesc" type="text" class="field-input" placeholder="简要说明这个流程的用途" />
        </div>
      </div>
    </Dialog>

    <!-- ================= 参数编辑 Dialog ================= -->
    <Dialog
      v-model:visible="paramDialogOpen"
      title="编辑节点参数"
      confirm-text="保存"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="saveParams"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">节点名称</label>
          <input v-model="paramLabel" type="text" class="field-input" placeholder="节点标签" />
        </div>
        <div class="field">
          <label class="field-label">参数（JSON）</label>
          <textarea v-model="paramJson" class="field-textarea mono" rows="8" spellcheck="false"></textarea>
        </div>
      </div>
    </Dialog>

    <!-- ================= 删除确认 Dialog ================= -->
    <Dialog
      v-model:visible="deleteDialogOpen"
      title="删除智能体流程"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-content">
        确定删除流程「{{ deleteTarget?.name }}」？此操作不可撤销。
      </div>
    </Dialog>
  </div>
</template>

<style scoped>
.flow-view {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* ================= 列表模式 ================= */
.list-pane {
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 20px;
  padding: 24px 28px 32px;
  overflow-y: auto;
}

.pane-hero {
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
  color: var(--primary);
  flex-shrink: 0;
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

.flow-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.flow-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.flow-card:hover {
  border-color: var(--primary);
}

.flow-card-main {
  flex: 1;
  min-width: 0;
}

.flow-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.flow-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.flow-desc {
  flex-shrink: 0;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.flow-meta {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-top: 4px;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.meta-item {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.flow-card-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.list-footer {
  padding-top: 4px;
}

/* ================= 编辑模式 ================= */
.edit-pane {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
  background: var(--card);
  flex-shrink: 0;
}

.name-input {
  width: 220px;
  height: var(--h-control-md);
  padding: 0 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.name-input:focus {
  border-color: var(--primary);
}

.toolbar-spacer {
  flex: 1;
}

.lib-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--border);
  background: var(--card);
  flex-shrink: 0;
  flex-wrap: wrap;
}

.lib-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--muted);
  flex-shrink: 0;
}

.lib-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.quick-add {
  flex-shrink: 0;
}

.arm-hint {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 14px;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(91, 140, 255, 0.08);
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.arm-cancel {
  margin-left: auto;
  padding: 2px 10px;
  font-family: inherit;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: color var(--duration-fast) var(--ease-standard), border-color var(--duration-fast) var(--ease-standard);
}

.arm-cancel:hover {
  color: var(--primary);
  border-color: var(--primary);
}

/* ================= 画布 ================= */
.canvas-wrap {
  flex: 1;
  min-height: 0;
  overflow: auto;
  position: relative;
  background: var(--bg-2);
}

.stage {
  position: relative;
  background-color: var(--bg-2);
  background-image: radial-gradient(circle, var(--border-strong) 1px, transparent 1px);
  background-size: 26px 26px;
}

.edges {
  position: absolute;
  top: 0;
  left: 0;
  z-index: 1;
  pointer-events: none;
  overflow: visible;
}

.edge-path {
  fill: none;
  stroke-width: 2.5;
  stroke-linecap: round;
  opacity: 0.85;
}

.edge-hit {
  fill: var(--primary);
  fill-opacity: 0;
  stroke: none;
  pointer-events: auto;
  cursor: pointer;
}

.edge-hit:hover {
  fill: var(--danger, #e5484d);
  fill-opacity: 0.9;
}

.edge-temp {
  fill: none;
  stroke-width: 2;
  stroke-dasharray: 6 5;
  stroke-linecap: round;
  opacity: 0.7;
}

.node-layer {
  position: absolute;
  inset: 0;
  z-index: 2;
}

.flow-node {
  position: absolute;
  width: 200px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.18);
  overflow: hidden;
  will-change: transform;
}

.flow-node:hover {
  border-color: var(--primary);
}

.node-head {
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 0 8px 0 12px;
  background: var(--card-2);
  border-bottom: 1px solid var(--border);
  cursor: grab;
  user-select: none;
  touch-action: none;
}

.node-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.node-del {
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
  transition: background var(--duration-fast) var(--ease-standard), color var(--duration-fast) var(--ease-standard);
}

.node-del:hover {
  background: var(--card);
  color: var(--danger, #e5484d);
}

.node-ports {
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px;
}

.node-port {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.port-dot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--muted);
  border: 2px solid var(--card);
  box-shadow: 0 0 0 1px var(--border-strong);
  cursor: crosshair;
  transition: transform var(--duration-fast) var(--ease-standard);
  flex-shrink: 0;
}

.port-dot:hover {
  transform: scale(1.25);
}

.port-label {
  font-size: var(--fs-xs);
  color: var(--muted);
  white-space: nowrap;
}

.node-foot {
  height: 26px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  border-top: 1px solid var(--border);
  font-size: var(--fs-xs);
  color: var(--muted);
}

.foot-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

/* 节点进入/离开动画 */
.node-enter-active {
  transition: opacity var(--duration-base) var(--ease-standard),
    transform var(--duration-base) var(--ease-emphasized);
}

.node-leave-active {
  transition: opacity var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.node-enter-from {
  opacity: 0;
  transform: scale(0.7);
}

.node-leave-to {
  opacity: 0;
  transform: scale(0.8);
}

/* ================= Dialog 表单 ================= */
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

.field-textarea {
  width: 100%;
  padding: 10px 12px;
  font-family: inherit;
  font-size: var(--fs-sm);
  line-height: 1.5;
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  resize: vertical;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.field-textarea:focus {
  border-color: var(--primary);
}

.mono {
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.dialog-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>