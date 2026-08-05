<script setup lang="ts">
/**
 * PinnedMemoryPanel 永久记忆管理面板
 *
 * 作为 SettingsPanel 的 "memory" tab 内容嵌入（不自带 BindSheet 容器）。
 * 列出所有永久记忆，支持新增 / 编辑 / 删除 / 清空。
 *
 * 后端命令：
 * - list_pinned_memories
 * - add_pinned_memory { content, category? }
 * - update_pinned_memory { id, content?, category? }
 * - delete_pinned_memory { id }
 * - clear_pinned_memories
 *
 * 永久记忆的语义：被加入后会注入到每轮对话的 [永久记忆] 段，
 * 不依赖 RAG 检索相关性，因此适合存放"始终要遵守/参考"的偏好与事实。
 */
import { ref, computed, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Button,
  Chips,
  Menu,
  Dialog,
  IconButton,
  Icon,
  useToast,
  type MenuItemOption,
} from './basic'
import type { PinnedMemory, PinnedMemorySource } from '../types'

const props = defineProps<{ open: boolean }>()

const { toast } = useToast()

// ---------- 数据 ----------
const memories = ref<PinnedMemory[]>([])
const loading = ref(false)

// 预设分类标签（点击可快速填入）
const categoryPresets: { key: string; label: string }[] = [
  { key: 'preference', label: '偏好' },
  { key: 'fact', label: '事实' },
  { key: 'instruction', label: '指令' },
  { key: 'identity', label: '身份' },
  { key: 'project', label: '项目' },
]

// 来源文案映射
const sourceLabels: Record<PinnedMemorySource, { text: string; glyph: string; accent: string }> = {
  manual: { text: '手动', glyph: 'edit', accent: 'var(--muted)' },
  user_request: { text: '对话添加', glyph: 'chat', accent: 'var(--primary)' },
  assistant: { text: '助手建议', glyph: 'spark', accent: '#a855f7' },
}

function sourceMeta(s: PinnedMemorySource) {
  return sourceLabels[s] ?? sourceLabels.manual
}

function categoryLabel(cat?: string | null): string {
  if (!cat) return '未分类'
  const hit = categoryPresets.find((p) => p.key === cat)
  return hit ? hit.label : cat
}

function formatDate(ts: number): string {
  if (!ts) return '—'
  try {
    const d = new Date(ts)
    const yyyy = d.getFullYear()
    const mm = String(d.getMonth() + 1).padStart(2, '0')
    const dd = String(d.getDate()).padStart(2, '0')
    const hh = String(d.getHours()).padStart(2, '0')
    const mi = String(d.getMinutes()).padStart(2, '0')
    return `${yyyy}-${mm}-${dd} ${hh}:${mi}`
  } catch {
    return '—'
  }
}

// ---------- 加载 ----------
async function refresh() {
  loading.value = true
  try {
    memories.value = await invoke<PinnedMemory[]>('list_pinned_memories')
  } catch (e) {
    toast({ content: `加载永久记忆失败：${e}`, type: 'error' })
    memories.value = []
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  refresh()
})

// 仅在面板首次打开时刷新（避免每次切换 tab 都请求）
let loadedOnce = false
watch(
  () => props.open,
  (v) => {
    if (v && !loadedOnce) {
      loadedOnce = true
      refresh()
    }
  },
)

const isEmpty = computed(() => !loading.value && memories.value.length === 0)

// ---------- 卡片菜单 ----------
const menuOpen = ref(false)
const menuTriggerEl = ref<HTMLElement | null>(null)
const menuTarget = ref<PinnedMemory | null>(null)

function openCardMenu(m: PinnedMemory, e: MouseEvent) {
  menuTarget.value = m
  menuTriggerEl.value = e.currentTarget as HTMLElement
  menuOpen.value = true
}

const menuItems = computed<MenuItemOption[]>(() => [
  { key: 'edit', label: '编辑', icon: 'edit' },
  { key: 'delete', label: '删除', icon: 'delete', danger: true, divided: true },
])

function onMenuSelect(item: MenuItemOption) {
  const m = menuTarget.value
  menuTarget.value = null
  if (!m) return
  if (item.key === 'edit') startEdit(m)
  else if (item.key === 'delete') {
    deleteTarget.value = m
    deleteDialogOpen.value = true
  }
}

// ---------- 新建 / 编辑 Dialog ----------
const dialogOpen = ref(false)
const editingId = ref<string | null>(null)

const draft = ref({
  content: '',
  category: '',
})

const dialogTitle = computed(() => (editingId.value ? '编辑永久记忆' : '新建永久记忆'))
const dialogConfirmText = computed(() => (editingId.value ? '保存' : '添加'))

function openCreate() {
  editingId.value = null
  draft.value = { content: '', category: '' }
  dialogOpen.value = true
}

function startEdit(m: PinnedMemory) {
  editingId.value = m.id
  draft.value = {
    content: m.content,
    category: m.category ?? '',
  }
  dialogOpen.value = true
}

function applyPresetCategory(key: string) {
  draft.value.category = key
}

async function confirmSave() {
  const content = draft.value.content.trim()
  if (!content) {
    toast({ content: '请输入记忆内容', type: 'warn' })
    return
  }
  if (content.length > 2000) {
    toast({ content: '内容过长（>2000 字符）', type: 'warn' })
    return
  }
  const category = draft.value.category.trim() || null
  try {
    if (editingId.value) {
      // update_pinned_memory: category 为 None 表示不变，空字符串表示清空
      // 这里前端语义：用户清空输入框 = 清空分类（Some("")）
      await invoke('update_pinned_memory', {
        id: editingId.value,
        content,
        category: draft.value.category.trim(),
      })
      toast({ content: '已更新永久记忆', type: 'success' })
    } else {
      await invoke('add_pinned_memory', {
        content,
        category: category ?? undefined,
      })
      toast({ content: '已添加永久记忆', type: 'success' })
    }
    dialogOpen.value = false
    await refresh()
  } catch (e) {
    toast({ content: `保存失败：${e}`, type: 'error' })
  }
}

// ---------- 删除单条 ----------
const deleteDialogOpen = ref(false)
const deleteTarget = ref<PinnedMemory | null>(null)

async function confirmDelete() {
  const m = deleteTarget.value
  if (!m) return
  try {
    await invoke('delete_pinned_memory', { id: m.id })
    toast({ content: '已删除该条永久记忆', type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `删除失败：${e}`, type: 'error' })
  } finally {
    deleteTarget.value = null
  }
}

// ---------- 清空全部 ----------
const clearDialogOpen = ref(false)

async function confirmClearAll() {
  try {
    await invoke('clear_pinned_memories')
    toast({ content: '已清空所有永久记忆', type: 'success' })
    await refresh()
  } catch (e) {
    toast({ content: `清空失败：${e}`, type: 'error' })
  } finally {
    clearDialogOpen.value = false
  }
}
</script>

<template>
  <section class="page memory-page">
    <header class="page-head">
      <div class="page-head-main">
        <h2 class="page-title">永久记忆</h2>
        <p class="page-sub">
          用户对话中要求"记住"的内容会永久注入到每轮上下文，不依赖检索相关性
        </p>
      </div>
      <Button variant="primary" size="sm" @click="openCreate">
        <template #icon><Icon name="plus" :size="16" /></template>
        新建
      </Button>
    </header>

    <!-- 说明卡片 -->
    <div class="hero-card">
      <div class="hero-mark"><Icon name="pin-filled" :size="22" /></div>
      <div class="hero-text">
        <div class="hero-title">什么是永久记忆？</div>
        <p class="hero-sub">
          与 RAG 历史检索不同，永久记忆会以 <code>[永久记忆]</code> 段始终注入到每轮对话的系统提示中。
          适合存放：用户偏好、身份信息、长期指令、项目背景等"始终要遵守/参考"的内容。
        </p>
        <p class="hero-sub hero-sub--hint">
          对话中可直接说"请记住我的工作邮箱是 xxx"，AI 会自动调用工具落盘到这里。
        </p>
      </div>
    </div>

    <!-- 列表区 -->
    <section class="section">
      <div class="section-head">
        <span class="section-title">全部记忆</span>
        <span v-if="memories.length" class="count-badge">{{ memories.length }}</span>
      </div>

      <!-- 空状态 -->
      <div v-if="isEmpty" class="empty-state">
        <div class="empty-illust"><Icon name="pin" :size="44" /></div>
        <p class="empty-text">还没有永久记忆</p>
        <p class="empty-hint">点击右上角"新建"添加，或在对话中告诉 AI "请记住..."</p>
      </div>

      <!-- 卡片列表 -->
      <div v-else class="memory-list">
        <article
          v-for="m in memories"
          :key="m.id"
          class="memory-card"
        >
          <div class="memory-card-main">
            <div class="memory-meta-row">
              <Chips
                v-if="m.category"
                :label="categoryLabel(m.category)"
                size="sm"
              />
              <span class="source-tag" :style="{ color: sourceMeta(m.source).accent }">
                <Icon :name="sourceMeta(m.source).glyph" :size="12" />
                {{ sourceMeta(m.source).text }}
              </span>
              <span class="memory-time">
                <Icon name="clock" :size="12" />
                {{ formatDate(m.created_at) }}
              </span>
            </div>
            <p class="memory-content">{{ m.content }}</p>
          </div>
          <IconButton
            size="sm"
            title="更多操作"
            @click.stop="(e) => openCardMenu(m, e)"
          ><Icon name="more-horizontal" :size="20" /></IconButton>
        </article>
      </div>
    </section>

    <!-- 危险操作：清空全部 -->
    <div v-if="memories.length" class="action-card action-card--danger">
      <div class="action-card-text">
        <span class="action-card-title">清空全部永久记忆</span>
        <span class="action-card-hint">删除所有条目，不可恢复</span>
      </div>
      <Button variant="danger" size="sm" @click="clearDialogOpen = true">
        清空全部
      </Button>
    </div>

    <!-- 卡片操作菜单 -->
    <Menu
      :visible="menuOpen"
      :items="menuItems"
      :trigger-ref="menuTriggerEl"
      placement="bottom-end"
      @update:visible="menuOpen = $event"
      @select="onMenuSelect"
    />

    <!-- 新建 / 编辑 Dialog -->
    <Dialog
      v-model:visible="dialogOpen"
      :title="dialogTitle"
      :confirm-text="dialogConfirmText"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmSave"
    >
      <div class="form-body">
        <div class="field">
          <label class="field-label">内容</label>
          <textarea
            v-model="draft.content"
            rows="5"
            class="field-input field-textarea"
            placeholder="例如：我的工作邮箱是 hr@effisuite.com"
            maxlength="2000"
          ></textarea>
          <div class="field-counter">{{ draft.content.length }} / 2000</div>
        </div>
        <div class="field">
          <label class="field-label">分类（可选）</label>
          <input
            v-model="draft.category"
            type="text"
            class="field-input"
            placeholder="自定义或选择下方预设"
          />
          <div class="preset-row">
            <Chips
              v-for="p in categoryPresets"
              :key="p.key"
              :label="p.label"
              size="sm"
              :selected="draft.category === p.key"
              @click="applyPresetCategory(p.key)"
            />
          </div>
        </div>
      </div>
    </Dialog>

    <!-- 删除确认 -->
    <Dialog
      v-model:visible="deleteDialogOpen"
      title="删除永久记忆？"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmDelete"
    >
      <div class="dialog-delete-content">
        <p>确定删除以下永久记忆？此操作不可撤销。</p>
        <p class="dialog-delete-preview">{{ deleteTarget?.content }}</p>
      </div>
    </Dialog>

    <!-- 清空全部确认 -->
    <Dialog
      v-model:visible="clearDialogOpen"
      title="清空全部永久记忆？"
      danger
      confirm-text="全部清空"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmClearAll"
    >
      <div class="dialog-delete-content">
        将删除全部 {{ memories.length }} 条永久记忆，且无法撤销。建议在清空前导出备份。
      </div>
    </Dialog>
  </section>
</template>

<style scoped>
.memory-page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}

/* ---------- 页头 ---------- */
.page-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 4px;
}

.page-head-main {
  min-width: 0;
}

.page-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text);
}

.page-sub {
  margin: 4px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 说明卡片 ---------- */
.hero-card {
  display: flex;
  gap: 14px;
  padding: 16px 18px;
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.12), rgba(74, 126, 255, 0.02));
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.hero-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border-radius: var(--radius-md);
  background: var(--card-2);
  color: var(--primary);
  flex-shrink: 0;
}

.hero-text {
  min-width: 0;
  flex: 1;
}

.hero-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
  margin-bottom: 4px;
}

.hero-sub {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.55;
}

.hero-sub--hint {
  margin-top: 6px;
  color: var(--primary);
}

.hero-sub code {
  padding: 1px 6px;
  font-family: 'JetBrains Mono', 'Consolas', monospace;
  font-size: 0.85em;
  background: var(--card-2);
  border-radius: var(--radius-md);
  color: var(--text);
}

/* ---------- 列表区 ---------- */
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
  border-radius: var(--radius-full);
  line-height: 1.4;
}

/* ---------- 空状态 ---------- */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 36px 20px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  color: var(--muted);
}

.empty-illust {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background: var(--card-2);
  color: var(--muted);
  margin-bottom: 12px;
}

.empty-text {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 500;
  color: var(--text);
}

.empty-hint {
  margin: 6px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  text-align: center;
  line-height: 1.5;
  max-width: 320px;
}

/* ---------- 记忆卡片 ---------- */
.memory-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.memory-card {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 12px 14px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard);
}

.memory-card:hover {
  border-color: var(--primary);
  background: var(--card-2);
}

.memory-card-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.memory-meta-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.source-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xs);
  font-weight: 500;
  line-height: 1;
}

.memory-time {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1;
}

.memory-content {
  margin: 0;
  font-size: var(--fs-base);
  color: var(--text);
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
}

/* ---------- 危险操作卡片 ---------- */
.action-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.action-card--danger {
  border-color: rgba(220, 70, 70, 0.4);
  background: rgba(220, 70, 70, 0.04);
}

.action-card-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.action-card-title {
  font-size: var(--fs-base);
  font-weight: 500;
  color: var(--text);
}

.action-card-hint {
  font-size: var(--fs-sm);
  color: var(--muted);
}

/* ---------- 表单 ---------- */
.form-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 4px 2px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text);
}

.field-input {
  width: 100%;
  padding: 9px 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
  box-sizing: border-box;
}

.field-input:focus {
  border-color: var(--primary);
}

.field-textarea {
  resize: vertical;
  min-height: 96px;
  line-height: 1.5;
  font-family: inherit;
}

.field-counter {
  align-self: flex-end;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.preset-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 4px;
}

/* ---------- Dialog 内容 ---------- */
.dialog-delete-content {
  font-size: var(--fs-base);
  color: var(--text);
  line-height: 1.55;
}

.dialog-delete-content p {
  margin: 0 0 8px;
}

.dialog-delete-preview {
  padding: 10px 12px;
  background: var(--card-2);
  border-radius: var(--radius-md);
  font-size: var(--fs-sm);
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
}
</style>
