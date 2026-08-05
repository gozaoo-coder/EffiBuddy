<script setup lang="ts">
/**
 * AsrHistoryTab —— ASR 历史记录视图
 *
 * 全流程管线（与用户动画规则一致）：
 *   挂载 → 加载 listRecords → 列表 stagger 进入
 *   搜索/筛选 → 列表 crossfade 替换（key=query+filter 触发 Transition）
 *   点击记录 → 右侧详情面板 slide-in（master-detail 同屏共存）
 *   切换记录 → 详情 crossfade（旧详情淡出、新详情淡入，避免跳变）
 *   删除 → 列表项 fade-out + 高度收缩，详情面板回到空状态
 *   中断（切走页签）：onBeforeUnmount 取消 record-updated 订阅；列表状态在
 *     module-level 单例中保留，返回时直接复用，无需重新加载。
 *
 * 架构（减少上帝文件）：
 *   - 业务逻辑全部下沉到 useAsr composable（list/search/delete/update/generateSummary）
 *   - 本组件仅负责视图渲染 + 动画 + 本地 UI 状态（选中 id / 搜索词 / 筛选）
 *   - 详情面板拆为 <template> 内的局部片段，避免再拆子组件引入过多文件
 */
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { animate } from 'animejs'
import Icon from '../Icon.vue'
import { Button, Dropdown, Dialog, useToast } from '../basic'
import { useAsr } from '../../composables/useAsr'
import { useAnimeTransition } from '../../composables/useAnimeTransition'
import type {
  AsrRecord,
  AsrSource,
  AsrStatus,
  AsrRecordUpdatedPayload,
  TabItem,
} from '../../types'

defineOptions({ name: 'AsrHistoryTab' })

const props = defineProps<{
  tab: TabItem
}>()

const emit = defineEmits<{
  (e: 'update:status', status: TabItem['status']): void
}>()

const { toast } = useToast()
const {
  records,
  loading,
  listRecords,
  getRecord,
  deleteRecord,
  searchRecords,
  generateSummary,
  updateRecord,
  onRecordUpdated,
} = useAsr()

// ============= 本地 UI 状态 =============
const keyword = ref('')
const sourceFilter = ref<AsrSource | ''>('')
const statusFilter = ref<AsrStatus | ''>('')
const selectedId = ref<string | null>(null)
/** 完整记录（含 transcript），从 getRecord 拉取 */
const detailRecord = ref<AsrRecord | null>(null)
const detailLoading = ref(false)
const deleteDialog = ref(false)
const deleting = ref(false)
/** 编辑模式 */
const editing = ref(false)
const editTitle = ref('')
const editTags = ref('')
/** 摘要生成中 */
const summarizing = ref(false)

// ============= 筛选选项 =============
const sourceOptions = [
  { value: '', label: '全部来源' },
  { value: 'streaming', label: '流式录入' },
  { value: 'upload', label: '文件上传' },
]

const statusOptions = [
  { value: '', label: '全部状态' },
  { value: 'completed', label: '已完成' },
  { value: 'transcribed', label: '已转写' },
  { value: 'summarizing', label: '摘要中' },
  { value: 'pending', label: '等待中' },
  { value: 'failed', label: '失败' },
]

// ============= 计算属性 =============
/** 列表 Transition 的 key：搜索词/筛选变化时触发 crossfade */
const listKey = computed(
  () => `${keyword.value.trim()}|${sourceFilter.value}|${statusFilter.value}`,
)

/** 本地过滤后的列表（records 已由 useAsr 管理，这里做即时过滤兜底） */
const filteredRecords = computed(() => {
  const q = keyword.value.trim().toLowerCase()
  return records.value.filter((r) => {
    if (sourceFilter.value && r.source !== sourceFilter.value) return false
    if (statusFilter.value && r.status !== statusFilter.value) return false
    if (q) {
      const hay = `${r.title} ${r.summary ?? ''} ${r.tags.join(' ')}`.toLowerCase()
      if (!hay.includes(q)) return false
    }
    return true
  })
})

const hasRecords = computed(() => records.value.length > 0)
const hasResults = computed(() => filteredRecords.value.length > 0)

// ============= 动画 =============
/** 列表 crossfade：搜索/筛选变化时整列替换 */
const { onEnter: onListEnter, onLeave: onListLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [8, 0],
    duration: 220,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    duration: 140,
    ease: 'inOut(2)',
  },
})

/** 详情面板切换：crossfade（避免旧详情跳变为新详情） */
const { onEnter: onDetailEnter, onLeave: onDetailLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateX: [16, 0],
    duration: 280,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    duration: 160,
    ease: 'inOut(2)',
  },
})

/** 列表项 stagger 进入：首次加载 / 刷新后逐项淡入 */
const listRef = ref<HTMLElement | null>(null)
function animateListStagger(): void {
  const container = listRef.value
  if (!container) return
  const items = container.querySelectorAll<HTMLElement>('.record-item')
  if (items.length === 0) return
  // 先锁定初始态，避免首帧闪现
  items.forEach((el) => {
    el.style.opacity = '0'
    el.style.transform = 'translateY(10px)'
  })
  // 逐项触发：anime.js v4 的 delay 不支持函数值，用 setTimeout 实现 stagger
  items.forEach((el, i) => {
    setTimeout(() => {
      animate(el, {
        opacity: [0, 1],
        translateY: ['10px', 0],
        duration: 260,
        ease: 'out(3)',
        onComplete: () => {
          el.style.opacity = ''
          el.style.transform = ''
        },
      })
    }, i * 40)
  })
}

// ============= 加载 / 搜索 =============
let searchTimer: number | null = null

async function loadAll(): Promise<void> {
  emit('update:status', 'loading')
  try {
    await listRecords()
    emit('update:status', 'idle')
    await nextTick()
    animateListStagger()
  } catch (e) {
    toast({ content: `加载历史失败：${e}`, type: 'error' })
    emit('update:status', 'error')
  }
}

/** 关键词输入防抖：250ms 后触发后端搜索（带 source/status 过滤） */
function onKeywordInput(): void {
  if (searchTimer !== null) window.clearTimeout(searchTimer)
  searchTimer = window.setTimeout(async () => {
    const q = keyword.value.trim()
    if (!q && !sourceFilter.value && !statusFilter.value) {
      // 无任何过滤条件：直接全量加载
      await loadAll()
      return
    }
    try {
      await searchRecords({
        keyword: q || null,
        source: sourceFilter.value || null,
        status: statusFilter.value || null,
      })
      await nextTick()
      animateListStagger()
    } catch (e) {
      toast({ content: `搜索失败：${e}`, type: 'error' })
    }
  }, 250)
}

function onSourceChange(v: string | number): void {
  sourceFilter.value = (v || '') as AsrSource | ''
  onKeywordInput()
}

function onStatusChange(v: string | number): void {
  statusFilter.value = (v || '') as AsrStatus | ''
  onKeywordInput()
}

function clearFilters(): void {
  keyword.value = ''
  sourceFilter.value = ''
  statusFilter.value = ''
  void loadAll()
}

// ============= 详情 =============
async function selectRecord(id: string): Promise<void> {
  if (selectedId.value === id) return
  selectedId.value = id
  detailRecord.value = null
  detailLoading.value = true
  editing.value = false
  try {
    const full = await getRecord(id)
    detailRecord.value = full
  } catch (e) {
    toast({ content: `加载详情失败：${e}`, type: 'error' })
    selectedId.value = null
  } finally {
    detailLoading.value = false
  }
}

function closeDetail(): void {
  selectedId.value = null
  detailRecord.value = null
  editing.value = false
}

// ============= 编辑 =============
function startEdit(): void {
  if (!detailRecord.value) return
  editTitle.value = detailRecord.value.title
  editTags.value = detailRecord.value.tags.join(', ')
  editing.value = true
}

async function saveEdit(): Promise<void> {
  if (!detailRecord.value) return
  const title = editTitle.value.trim()
  if (!title) {
    toast({ content: '标题不能为空', type: 'warn' })
    return
  }
  const tags = editTags.value
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean)
  try {
    const updated = await updateRecord(detailRecord.value.id, { title, tags })
    detailRecord.value = updated
    editing.value = false
    toast({ content: '已保存', type: 'success' })
  } catch (e) {
    toast({ content: `保存失败：${e}`, type: 'error' })
  }
}

function cancelEdit(): void {
  editing.value = false
}

// ============= 摘要 =============
async function regenerateSummary(): Promise<void> {
  if (!detailRecord.value) return
  summarizing.value = true
  try {
    const summary = await generateSummary(detailRecord.value.id)
    detailRecord.value = { ...detailRecord.value, summary }
    toast({ content: summary ? '摘要已生成' : '摘要为空', type: 'success' })
  } catch (e) {
    toast({ content: `摘要生成失败：${e}`, type: 'error' })
  } finally {
    summarizing.value = false
  }
}

// ============= 删除 =============
function confirmDelete(): void {
  deleteDialog.value = true
}

async function doDelete(): Promise<void> {
  if (!detailRecord.value) return
  deleting.value = true
  const id = detailRecord.value.id
  try {
    await deleteRecord(id)
    // 列表项由 records 响应式自动移除；关闭详情
    closeDetail()
    deleteDialog.value = false
    toast({ content: '已删除记录', type: 'success' })
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
  } finally {
    deleting.value = false
  }
}

// ============= 复制 =============
async function copyTranscript(): Promise<void> {
  if (!detailRecord.value?.transcript) return
  try {
    await navigator.clipboard.writeText(detailRecord.value.transcript)
    toast({ content: '转写文本已复制', type: 'success' })
  } catch {
    toast({ content: '复制失败，请手动选择文本', type: 'error' })
  }
}

// ============= 格式化 =============
function formatDate(iso: string): string {
  const d = new Date(iso)
  if (isNaN(d.getTime())) return iso
  const now = new Date()
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  const hh = d.getHours().toString().padStart(2, '0')
  const mm = d.getMinutes().toString().padStart(2, '0')
  if (sameDay) return `今天 ${hh}:${mm}`
  const mo = (d.getMonth() + 1).toString().padStart(2, '0')
  const dd = d.getDate().toString().padStart(2, '0')
  return `${mo}-${dd} ${hh}:${mm}`
}

function formatDuration(ms: number): string {
  if (!ms || ms <= 0) return '--'
  const s = Math.floor(ms / 1000)
  const mm = Math.floor(s / 60).toString().padStart(2, '0')
  const ss = (s % 60).toString().padStart(2, '0')
  return `${mm}:${ss}`
}

function statusLabel(status: AsrStatus): string {
  const map: Record<AsrStatus, string> = {
    pending: '等待中',
    transcribing: '转写中',
    transcribed: '已转写',
    summarizing: '摘要中',
    completed: '已完成',
    failed: '失败',
  }
  return map[status] ?? status
}

function sourceLabel(source: AsrSource): string {
  return source === 'streaming' ? '流式' : '上传'
}

// ============= 事件订阅 =============
let unsubUpdated: (() => void) | null = null

// ============= 生命周期 =============
function init(): void {
  emit('update:status', 'idle')
  // 订阅记录更新事件：详情打开时局部刷新
  unsubUpdated = onRecordUpdated((p: AsrRecordUpdatedPayload) => {
    if (selectedId.value === p.record_id) {
      // 重新拉取完整详情
      void getRecord(p.record_id).then((full) => {
        if (full && selectedId.value === p.record_id) {
          detailRecord.value = full
        }
      })
    }
  })
  // 首次加载列表
  void loadAll()
}

function destroy(): void {
  /* 状态在 module-level 单例保留，返回时复用 */
}

onMounted(() => {
  init()
})

onBeforeUnmount(() => {
  destroy()
  unsubUpdated?.()
  if (searchTimer !== null) window.clearTimeout(searchTimer)
})

// 列表数据变化后触发 stagger（如搜索结果到达）
watch(
  () => records.value.length,
  async () => {
    await nextTick()
    animateListStagger()
  },
)

defineExpose({ init, destroy })
</script>

<template>
  <div class="asr-tab asr-history-tab">
    <!-- ============= 工具栏 ============= -->
    <div class="history-toolbar">
      <div class="search-box">
        <Icon name="search" :size="16" class="search-icon" />
        <input
          v-model="keyword"
          class="search-input"
          type="text"
          placeholder="搜索标题、摘要、标签…"
          autocomplete="off"
          @input="onKeywordInput"
        />
        <button
          v-if="keyword"
          type="button"
          class="search-clear"
          @click="keyword = ''; onKeywordInput()"
        >
          <Icon name="close" :size="14" />
        </button>
      </div>

      <div class="filter-group">
        <Dropdown
          :options="sourceOptions"
          :model-value="sourceFilter"
          size="sm"
          width="auto"
          @update:model-value="onSourceChange"
        />
        <Dropdown
          :options="statusOptions"
          :model-value="statusFilter"
          size="sm"
          width="auto"
          @update:model-value="onStatusChange"
        />
        <Button
          v-if="keyword || sourceFilter || statusFilter"
          variant="text"
          size="sm"
          @click="clearFilters"
        >
          清除筛选
        </Button>
      </div>

      <Button
        variant="normal"
        size="sm"
        :loading="loading"
        :icon-only="true"
        shape="circle"
        title="刷新"
        @click="loadAll"
      >
        <template #icon><Icon name="refresh" :size="16" /></template>
      </Button>
    </div>

    <!-- ============= 主内容区：列表 + 详情 ============= -->
    <div class="history-body">
      <!-- 列表列 -->
      <div class="history-list-col">
        <!-- 空状态 -->
        <div v-if="!hasRecords && !loading" class="history-empty">
          <div class="history-empty-icon"><Icon name="book" :size="40" /></div>
          <p class="history-empty-text">暂无历史记录</p>
          <p class="history-empty-hint">开始流式录入或上传文件后，记录会出现在这里</p>
        </div>

        <!-- 无搜索结果 -->
        <div v-else-if="!hasResults && !loading" class="history-empty">
          <div class="history-empty-icon"><Icon name="search" :size="40" /></div>
          <p class="history-empty-text">未找到匹配记录</p>
          <p class="history-empty-hint">尝试更换关键词或清除筛选条件</p>
        </div>

        <!-- 列表 -->
        <div v-else ref="listRef" class="record-list">
          <Transition :css="false" @enter="onListEnter" @leave="onListLeave">
            <div :key="listKey" class="record-list-inner">
              <button
                v-for="r in filteredRecords"
                :key="r.id"
                type="button"
                class="record-item"
                :class="{ active: selectedId === r.id }"
                @click="selectRecord(r.id)"
              >
                <div class="record-item-head">
                  <span class="record-title">{{ r.title || '无标题' }}</span>
                  <span
                    class="record-status"
                    :class="`status-${r.status}`"
                  >{{ statusLabel(r.status) }}</span>
                </div>
                <p class="record-preview">
                  {{ r.summary || r.transcript || '（无文本内容）' }}
                </p>
                <div class="record-item-meta">
                  <span class="meta-source" :class="`source-${r.source}`">
                    <Icon :name="r.source === 'streaming' ? 'mic' : 'attachment'" :size="12" />
                    {{ sourceLabel(r.source) }}
                  </span>
                  <span class="meta-duration">
                    <Icon name="clock" :size="12" />
                    {{ formatDuration(r.duration_ms) }}
                  </span>
                  <span class="meta-date">{{ formatDate(r.created_at) }}</span>
                </div>
              </button>
            </div>
          </Transition>
        </div>
      </div>

      <!-- 详情列 -->
      <div class="history-detail-col">
        <!-- 空状态：未选中 -->
        <div v-if="!selectedId" class="detail-empty">
          <div class="detail-empty-icon"><Icon name="book" :size="48" /></div>
          <p class="detail-empty-text">选择左侧记录查看详情</p>
        </div>

        <!-- 加载中 -->
        <div v-else-if="detailLoading" class="detail-loading">
          <Icon name="loader" :size="28" class="spin-icon" />
          <span>加载中…</span>
        </div>

        <!-- 详情内容：crossfade 切换 -->
        <Transition v-else-if="detailRecord" :css="false" @enter="onDetailEnter" @leave="onDetailLeave" mode="out-in">
          <div :key="detailRecord.id" class="detail-content">
            <!-- 详情头部 -->
            <div class="detail-header">
              <button type="button" class="detail-back" @click="closeDetail">
                <Icon name="arrow-left" :size="18" />
              </button>
              <div class="detail-title-wrap">
                <input
                  v-if="editing"
                  v-model="editTitle"
                  class="detail-title-input"
                  type="text"
                  placeholder="记录标题"
                />
                <h3 v-else class="detail-title">
                  {{ detailRecord.title || '无标题' }}
                </h3>
              </div>
            </div>

            <!-- 元信息条 -->
            <div class="detail-meta-bar">
              <span class="meta-chip" :class="`source-${detailRecord.source}`">
                <Icon :name="detailRecord.source === 'streaming' ? 'mic' : 'attachment'" :size="13" />
                {{ sourceLabel(detailRecord.source) }}
              </span>
              <span class="meta-chip" :class="`status-${detailRecord.status}`">
                {{ statusLabel(detailRecord.status) }}
              </span>
              <span class="meta-chip">
                <Icon name="clock" :size="13" />
                {{ formatDuration(detailRecord.duration_ms) }}
              </span>
              <span class="meta-chip">{{ formatDate(detailRecord.created_at) }}</span>
              <span v-if="detailRecord.language" class="meta-chip">
                {{ detailRecord.language }}
              </span>
            </div>

            <!-- 标签 -->
            <div class="detail-tags">
              <template v-if="editing">
                <input
                  v-model="editTags"
                  class="tags-input"
                  type="text"
                  placeholder="标签（逗号分隔）"
                />
              </template>
              <template v-else>
                <span v-for="tag in detailRecord.tags" :key="tag" class="tag-chip">{{ tag }}</span>
                <span v-if="detailRecord.tags.length === 0" class="tags-empty">无标签</span>
              </template>
            </div>

            <!-- 操作按钮 -->
            <div class="detail-actions">
              <template v-if="editing">
                <Button variant="primary" size="sm" @click="saveEdit">保存</Button>
                <Button variant="normal" size="sm" @click="cancelEdit">取消</Button>
              </template>
              <template v-else>
                <Button variant="normal" size="sm" @click="startEdit">
                  <template #icon><Icon name="edit" :size="14" /></template>
                  编辑
                </Button>
                <Button
                  variant="normal"
                  size="sm"
                  :loading="summarizing"
                  @click="regenerateSummary"
                >
                  <template #icon><Icon name="sparkles" :size="14" /></template>
                  {{ detailRecord.summary ? '重新摘要' : '生成摘要' }}
                </Button>
                <Button variant="normal" size="sm" @click="copyTranscript">
                  <template #icon><Icon name="copy" :size="14" /></template>
                  复制
                </Button>
                <Button variant="danger" size="sm" @click="confirmDelete">
                  <template #icon><Icon name="delete" :size="14" /></template>
                  删除
                </Button>
              </template>
            </div>

            <!-- 转写文本 -->
            <div class="detail-section">
              <div class="detail-section-head">
                <Icon name="book" :size="16" />
                <span>转写文本</span>
              </div>
              <div class="detail-transcript">
                {{ detailRecord.transcript || '（无转写文本）' }}
              </div>
            </div>

            <!-- AI 摘要 -->
            <div v-if="detailRecord.summary" class="detail-section">
              <div class="detail-section-head summary-head">
                <Icon name="sparkles" :size="16" />
                <span>AI 摘要</span>
              </div>
              <div class="detail-summary">{{ detailRecord.summary }}</div>
            </div>

            <!-- 错误信息 -->
            <div v-if="detailRecord.error_message" class="detail-error">
              <Icon name="warning" :size="16" />
              <span>{{ detailRecord.error_message }}</span>
            </div>
          </div>
        </Transition>
      </div>
    </div>

    <!-- 删除确认对话框 -->
    <Dialog
      :visible="deleteDialog"
      title="删除此记录？"
      content="将永久删除该转写记录及其摘要，操作不可撤销。"
      confirm-text="删除"
      danger
      @confirm="doDelete"
      @cancel="deleteDialog = false"
    />
  </div>
</template>

<style scoped>
.asr-tab {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--bg);
  gap: var(--space-4);
}

/* ============= 工具栏 ============= */
.history-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-5);
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.search-box {
  position: relative;
  display: flex;
  align-items: center;
  flex: 1;
  max-width: 320px;
}

.search-icon {
  position: absolute;
  left: 10px;
  color: var(--muted);
  pointer-events: none;
}

.search-input {
  width: 100%;
  padding: 7px 32px 7px 32px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--card);
  color: var(--text);
  font-family: inherit;
  font-size: var(--fs-sm);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.search-input:focus {
  border-color: var(--primary);
}

.search-clear {
  position: absolute;
  right: 6px;
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
}

.search-clear:hover {
  background: var(--card-2);
  color: var(--text);
}

.filter-group {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

/* ============= 主内容区 ============= */
.history-body {
  display: flex;
  flex: 1;
  min-height: 0;
  gap: 1px;
  background: var(--border);
}

.history-list-col {
  width: 42%;
  min-width: 280px;
  max-width: 480px;
  background: var(--bg);
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}

.history-detail-col {
  flex: 1;
  min-width: 0;
  background: var(--bg);
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}

/* ============= 空状态 ============= */
.history-empty,
.detail-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  flex: 1;
  padding: var(--space-8);
  text-align: center;
}

.history-empty-icon,
.detail-empty-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 72px;
  height: 72px;
  border-radius: var(--radius-full);
  background: var(--card-2);
  color: var(--muted);
  margin-bottom: var(--space-2);
}

.history-empty-text,
.detail-empty-text {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 500;
  color: var(--text);
}

.history-empty-hint {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  max-width: 280px;
}

/* ============= 记录列表 ============= */
.record-list {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-2);
}

.record-list-inner {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.record-item {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  width: 100%;
  padding: var(--space-3) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  cursor: pointer;
  text-align: left;
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard);
}

.record-item:hover {
  border-color: var(--primary);
  background: var(--card-2);
}

.record-item.active {
  border-color: var(--primary);
  background: color-mix(in srgb, var(--primary) 8%, var(--card));
}

.record-item-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}

.record-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.record-status {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  padding: 1px 6px;
  border-radius: var(--radius-full);
  font-weight: 500;
  line-height: 1.5;
}

.status-completed {
  background: rgba(62, 207, 142, 0.14);
  color: var(--success);
}

.status-transcribed {
  background: rgba(74, 126, 255, 0.14);
  color: var(--primary);
}

.status-summarizing,
.status-transcribing,
.status-pending {
  background: rgba(245, 158, 11, 0.14);
  color: var(--warning);
}

.status-failed {
  background: rgba(255, 92, 92, 0.14);
  color: var(--danger);
}

.record-preview {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.record-item-meta {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--fs-xs);
  color: var(--muted);
}

.meta-source,
.meta-duration {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.source-streaming {
  color: var(--primary);
}

.source-upload {
  color: var(--success);
}

.meta-date {
  margin-left: auto;
}

/* ============= 详情面板 ============= */
.detail-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-3);
  flex: 1;
  color: var(--muted);
  font-size: var(--fs-sm);
}

.spin-icon {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.detail-content {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: var(--space-5) var(--space-6);
  flex: 1;
  min-height: 0;
}

.detail-header {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-shrink: 0;
}

.detail-back {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--card);
  color: var(--text);
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard);
}

.detail-back:hover {
  background: var(--card-2);
}

.detail-title-wrap {
  flex: 1;
  min-width: 0;
}

.detail-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.detail-title-input {
  width: 100%;
  padding: 6px 10px;
  border: 1px solid var(--primary);
  border-radius: var(--radius-md);
  background: var(--card);
  color: var(--text);
  font-family: inherit;
  font-size: var(--fs-md);
  font-weight: 600;
  outline: none;
}

/* 元信息条 */
.detail-meta-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-2);
  flex-shrink: 0;
}

.meta-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: var(--radius-full);
  background: var(--card-2);
  color: var(--muted);
  font-size: var(--fs-xs);
  font-weight: 500;
}

/* 标签 */
.detail-tags {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-2);
  flex-shrink: 0;
}

.tag-chip {
  padding: 3px 10px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--primary) 12%, transparent);
  color: var(--primary);
  font-size: var(--fs-xs);
  font-weight: 500;
}

.tags-empty {
  font-size: var(--fs-xs);
  color: var(--muted);
}

.tags-input {
  width: 100%;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--card);
  color: var(--text);
  font-family: inherit;
  font-size: var(--fs-sm);
  outline: none;
}

.tags-input:focus {
  border-color: var(--primary);
}

/* 操作按钮 */
.detail-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
  flex-shrink: 0;
}

/* 详情区段 */
.detail-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.detail-section-head {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.detail-section-head.summary-head {
  color: var(--primary);
}

.detail-transcript,
.detail-summary {
  font-size: var(--fs-base);
  line-height: 1.75;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--space-4) var(--space-5);
  max-height: 320px;
  overflow-y: auto;
}

.detail-error {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-lg);
  background: rgba(255, 92, 92, 0.08);
  border: 1px solid rgba(255, 92, 92, 0.3);
  color: var(--danger);
  font-size: var(--fs-sm);
}
</style>
