<script setup lang="ts">
/**
 * SideNav 左侧抽屉导航组件（Kimi 风格）
 * 从左滑入的抽屉，包含：
 * - 顶部 4 个功能入口（设置 / 定时任务 / 设备管理 / 模型配置）
 * - 搜索框 + 历史会话列表（置顶分组可折叠）
 * - 长按 / 右键弹出 Menu（改名 / 置顶 / 删除）
 * - 底部「新建聊天」按钮
 *
 * 容器复用 BindSheet side="left"，自带遮罩、ESC、滑入动画。
 */
import { ref, computed, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { BindSheet, Button, Menu, Dialog, Icon, useToast, type MenuItemOption } from './basic'
import type { ConversationMeta, SearchHit } from '../types'

const props = defineProps<{
  /** 当前选中的会话 id，用于列表高亮 */
  activeId?: string | null
}>()

const emit = defineEmits<{
  (e: 'open-settings'): void
  (e: 'open-scheduled-tasks'): void
  (e: 'open-skills'): void
  (e: 'open-device'): void
  (e: 'open-model-config'): void
  (e: 'select-conversation', id: string | null): void
}>()

const open = defineModel<boolean>('open', { default: false })

const { toast } = useToast()

// ---------- 会话列表 ----------
const conversations = ref<ConversationMeta[]>([])
const loading = ref(false)

const pinnedConversations = computed(() => conversations.value.filter((c) => c.pinned))
const regularConversations = computed(() => conversations.value.filter((c) => !c.pinned))

const pinnedCollapsed = ref(false)

// ---------- 搜索 ----------
const searchQuery = ref('')
const searchResults = ref<SearchHit[]>([])
let searchTimer: number | null = null

watch(searchQuery, (q) => {
  if (searchTimer) {
    clearTimeout(searchTimer)
    searchTimer = null
  }
  const trimmed = q.trim()
  if (!trimmed) {
    searchResults.value = []
    return
  }
  searchTimer = window.setTimeout(async () => {
    try {
      searchResults.value = await invoke<SearchHit[]>('search_conversations', { query: trimmed })
    } catch (e) {
      console.warn('search_conversations failed', e)
      searchResults.value = []
    }
  }, 300)
})

// ---------- 长按 / 右键菜单 ----------
const menuVisible = ref(false)
const menuPosition = ref<{ x: number; y: number } | null>(null)
const menuTargetConv = ref<ConversationMeta | null>(null)

let longPressTimer: number | null = null

function onItemPointerDown(e: PointerEvent, conv: ConversationMeta) {
  // 鼠标用 contextmenu，触摸用长按
  if (e.pointerType === 'mouse') return
  longPressTimer = window.setTimeout(() => {
    openItemMenu(conv, e.clientX, e.clientY)
  }, 500)
}

function onItemPointerUp() {
  if (longPressTimer) {
    clearTimeout(longPressTimer)
    longPressTimer = null
  }
}

function onItemContextMenu(e: MouseEvent, conv: ConversationMeta) {
  e.preventDefault()
  openItemMenu(conv, e.clientX, e.clientY)
}

function openItemMenu(conv: ConversationMeta, x: number, y: number) {
  menuPosition.value = { x, y }
  menuTargetConv.value = conv
  menuVisible.value = true
}

const itemMenuItems = computed<MenuItemOption[]>(() => {
  const c = menuTargetConv.value
  if (!c) return []
  return [
    { key: 'rename', label: '改名', icon: 'edit' },
    { key: 'pin', label: c.pinned ? '取消置顶' : '置顶', icon: c.pinned ? 'pin-filled' : 'pin' },
    { key: 'delete', label: '删除', icon: 'delete', danger: true, divided: true },
  ]
})

function onMenuSelect(item: MenuItemOption) {
  const conv = menuTargetConv.value
  menuTargetConv.value = null
  if (!conv) return
  switch (item.key) {
    case 'rename':
      startRename(conv)
      break
    case 'pin':
      togglePin(conv)
      break
    case 'delete':
      deleteTargetId.value = conv.id
      deleteDialogVisible.value = true
      break
  }
}

// ---------- 改名 ----------
const renameDialogVisible = ref(false)
const renameTargetConv = ref<ConversationMeta | null>(null)
const renameInput = ref('')

function startRename(conv: ConversationMeta) {
  renameTargetConv.value = conv
  renameInput.value = conv.title ?? ''
  renameDialogVisible.value = true
}

async function confirmRename() {
  const conv = renameTargetConv.value
  if (!conv) return
  const title = renameInput.value.trim()
  try {
    await invoke('rename_conversation', { id: conv.id, title })
    await refresh()
    toast({ content: '已重命名', type: 'success' })
  } catch (e) {
    toast({ content: `重命名失败：${e}`, type: 'error' })
  } finally {
    renameTargetConv.value = null
  }
}

// ---------- 置顶 ----------
async function togglePin(conv: ConversationMeta) {
  try {
    await invoke('toggle_pin_conversation', { id: conv.id, pinned: !conv.pinned })
    await refresh()
    toast({ content: conv.pinned ? '已取消置顶' : '已置顶', type: 'success' })
  } catch (e) {
    toast({ content: `操作失败：${e}`, type: 'error' })
  }
}

// ---------- 删除 ----------
const deleteDialogVisible = ref(false)
const deleteTargetId = ref<string | null>(null)

async function confirmDelete() {
  const id = deleteTargetId.value
  if (!id) return
  try {
    await invoke('delete_conversation', { id })
    await refresh()
    // 删除的是当前选中会话 → 切换到第一个或 null
    if (id === props.activeId) {
      const next = conversations.value[0]?.id ?? null
      emit('select-conversation', next)
    }
    toast({ content: '会话已删除', type: 'success' })
  } catch (e) {
    toast({ content: `删除会话失败：${e}`, type: 'error' })
  } finally {
    deleteTargetId.value = null
  }
}

// ---------- 新建聊天 ----------
function onNewChat() {
  emit('select-conversation', null)
}

// ---------- 工具函数 ----------
function displayTitle(conv: ConversationMeta): string {
  const t = conv.title?.trim()
  if (t) return t.length > 20 ? t.slice(0, 20) + '…' : t
  return '新对话'
}

function formatRelativeTime(ts: number): string {
  const diff = Date.now() - ts
  if (diff < 60000) return '刚刚'
  if (diff < 3600000) return `${Math.floor(diff / 60000)}分钟前`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}小时前`
  if (diff < 2592000000) return `${Math.floor(diff / 86400000)}天前`
  try {
    return new Date(ts).toLocaleDateString()
  } catch {
    return ''
  }
}

// ---------- 数据加载 ----------
async function refresh() {
  loading.value = true
  try {
    conversations.value = await invoke<ConversationMeta[]>('list_conversations')
  } catch (e) {
    console.warn('list_conversations failed', e)
    conversations.value = []
  } finally {
    loading.value = false
  }
}

// 抽屉打开时刷新列表
watch(open, (v) => {
  if (v) refresh()
})

onMounted(() => {
  refresh()
})

defineExpose({ refresh })
</script>

<template>
  <BindSheet v-model:visible="open" side="left" width="320px" title="EffiBuddy">
    <div class="sidenav-body">
      <!-- 顶部功能入口 -->
      <div class="sidenav-entries">
        <button type="button" class="entry-btn" @click="emit('open-settings')">
          <span class="entry-icon"><Icon name="settings" :size="20" /></span>
          <span class="entry-label">设置</span>
        </button>
        <button type="button" class="entry-btn" @click="emit('open-skills')">
          <span class="entry-icon"><Icon name="bolt" :size="20" /></span>
          <span class="entry-label">技能</span>
        </button>
        <button type="button" class="entry-btn" @click="emit('open-scheduled-tasks')">
          <span class="entry-icon"><Icon name="alarm" :size="20" /></span>
          <span class="entry-label">定时任务</span>
        </button>
        <button type="button" class="entry-btn" @click="emit('open-device')">
          <span class="entry-icon"><Icon name="device" :size="20" /></span>
          <span class="entry-label">设备管理</span>
        </button>
        <button type="button" class="entry-btn" @click="emit('open-model-config')">
          <span class="entry-icon"><Icon name="robot" :size="20" /></span>
          <span class="entry-label">模型配置</span>
        </button>
      </div>

      <!-- 搜索框 -->
      <div class="sidenav-search">
        <span class="search-icon"><Icon name="search" :size="18" /></span>
        <input
          v-model="searchQuery"
          type="text"
          class="search-input"
          placeholder="搜索会话..."
        />
      </div>

      <!-- 会话列表 / 搜索结果 -->
      <div class="sidenav-list">
        <!-- 搜索结果 -->
        <template v-if="searchQuery.trim() && searchResults.length > 0">
          <div class="list-section-title">搜索结果</div>
          <div
            v-for="hit in searchResults"
            :key="hit.conversation_id + hit.message_id"
            class="conv-item"
            :class="{ active: hit.conversation_id === props.activeId }"
            @click="emit('select-conversation', hit.conversation_id)"
          >
            <div class="conv-item-title">{{ hit.conversation_title || '新对话' }}</div>
            <div class="conv-item-snippet">{{ hit.snippet }}</div>
          </div>
        </template>

        <!-- 搜索无结果 -->
        <div v-else-if="searchQuery.trim() && searchResults.length === 0" class="empty-hint">
          未找到相关会话
        </div>

        <!-- 正常列表 -->
        <template v-else>
          <!-- 置顶分组 -->
          <template v-if="pinnedConversations.length > 0">
            <div
              class="list-section-title collapsible"
              @click="pinnedCollapsed = !pinnedCollapsed"
            >
              <span class="section-arrow"><Icon :name="pinnedCollapsed ? 'chevron-right' : 'chevron-down'" :size="14" /></span>
              <span>置顶 ({{ pinnedConversations.length }})</span>
            </div>
            <template v-if="!pinnedCollapsed">
              <div
                v-for="c in pinnedConversations"
                :key="c.id"
                class="conv-item"
                :class="{ active: c.id === props.activeId }"
                @click="emit('select-conversation', c.id)"
                @pointerdown="onItemPointerDown($event, c)"
                @pointerup="onItemPointerUp"
                @pointerleave="onItemPointerUp"
                @pointercancel="onItemPointerUp"
                @contextmenu="onItemContextMenu($event, c)"
              >
                <div class="conv-item-title">{{ displayTitle(c) }}</div>
                <div class="conv-item-meta">
                  {{ c.message_count }} 条 · {{ formatRelativeTime(c.updated_at) }}
                </div>
              </div>
            </template>
          </template>

          <!-- 普通会话 -->
          <div
            v-for="c in regularConversations"
            :key="c.id"
            class="conv-item"
            :class="{ active: c.id === props.activeId }"
            @click="emit('select-conversation', c.id)"
            @pointerdown="onItemPointerDown($event, c)"
            @pointerup="onItemPointerUp"
            @pointerleave="onItemPointerUp"
            @pointercancel="onItemPointerUp"
            @contextmenu="onItemContextMenu($event, c)"
          >
            <div class="conv-item-title">{{ displayTitle(c) }}</div>
            <div class="conv-item-meta">
              {{ c.message_count }} 条 · {{ formatRelativeTime(c.updated_at) }}
            </div>
          </div>

          <!-- 空状态 -->
          <div v-if="conversations.length === 0 && !loading" class="empty-hint">
            暂无会话，点击下方新建
          </div>
        </template>
      </div>

      <!-- 底部新建按钮 -->
      <div class="sidenav-footer">
        <Button variant="primary" block @click="onNewChat">
          <template #icon>＋</template>
          新建聊天
        </Button>
      </div>
    </div>
  </BindSheet>

  <!-- 列表项上下文菜单 -->
  <Menu
    v-model:visible="menuVisible"
    :items="itemMenuItems"
    :position="menuPosition"
    @select="onMenuSelect"
  />

  <!-- 改名对话框 -->
  <Dialog
    v-model:visible="renameDialogVisible"
    title="改名"
    confirm-text="保存"
    cancel-text="取消"
    @confirm="confirmRename"
  >
    <input
      v-model="renameInput"
      type="text"
      class="rename-input"
      placeholder="输入新标题（留空使用首条消息摘要）"
    />
  </Dialog>

  <!-- 删除确认对话框 -->
  <Dialog
    v-model:visible="deleteDialogVisible"
    title="删除会话"
    danger
    confirm-text="删除"
    cancel-text="取消"
    :close-on-click-overlay="false"
    @confirm="confirmDelete"
  >
    <div class="dialog-delete-content">确定删除该会话？此操作不可撤销。</div>
  </Dialog>
</template>

<style scoped>
.sidenav-body {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

/* 顶部功能入口 */
.sidenav-entries {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: 4px;
  padding: 8px 12px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--border);
}

.entry-btn {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 10px 4px;
  background: transparent;
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text);
  transition: background var(--duration-fast) var(--ease-standard);
}

.entry-btn:hover {
  background: var(--card-2);
}

.entry-icon {
  font-size: 22px;
  line-height: 1;
}

.entry-label {
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
}

/* 搜索框 */
.sidenav-search {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 12px 16px 8px;
  padding: 0 12px;
  height: var(--h-control-md);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  flex-shrink: 0;
}

.search-icon {
  font-size: 14px;
  color: var(--muted);
}

.search-input {
  flex: 1;
  border: none;
  background: transparent;
  outline: none;
  font-family: inherit;
  font-size: 14px;
  color: var(--text);
}

.search-input::placeholder {
  color: var(--muted);
}

/* 会话列表 */
.sidenav-list {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  padding: 4px 8px 8px;
}

.list-section-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
  padding: 10px 12px 6px;
  user-select: none;
}

.list-section-title.collapsible {
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 4px;
}

.list-section-title.collapsible:hover {
  color: var(--text);
}

.section-arrow {
  font-size: 10px;
}

/* 会话项 */
.conv-item {
  padding: 10px 12px;
  border-radius: var(--radius);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
  user-select: none;
}

.conv-item:hover {
  background: var(--card-2);
}

.conv-item:active {
  background: var(--border);
}

.conv-item.active {
  background: rgba(74, 126, 255, 0.12);
}

.conv-item.active .conv-item-title {
  color: var(--primary);
}

.conv-item-title {
  font-size: 14px;
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.conv-item-meta {
  font-size: 12px;
  color: var(--muted);
  margin-top: 3px;
}

.conv-item-snippet {
  font-size: 12px;
  color: var(--muted);
  margin-top: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.empty-hint {
  padding: 24px 16px;
  text-align: center;
  font-size: 13px;
  color: var(--muted);
}

/* 底部新建按钮 */
.sidenav-footer {
  flex-shrink: 0;
  padding: 12px 16px 16px;
  border-top: 1px solid var(--border);
}

/* 改名输入框 */
.rename-input {
  width: 100%;
  height: var(--h-control-md);
  padding: 0 12px;
  font-family: inherit;
  font-size: 14px;
  color: var(--text);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  outline: none;
  transition: border-color var(--duration-fast) var(--ease-standard);
  margin: 4px 0;
}

.rename-input:focus {
  border-color: var(--primary);
}

.dialog-delete-content {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}
</style>
