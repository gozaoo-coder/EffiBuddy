<script setup lang="ts">
/**
 * HistoryRail 第二栏：历史记录栏
 *
 * 从上到下：
 *  1. 新建聊天按钮
 *  2. 搜索框
 *  3. 置顶（可折叠）
 *  4. 文件夹（用户自行对会话分类，localStorage 持久化）
 *  5. 会话列表（可按文件夹筛选）
 *
 * 会话右键/长按菜单：改名 / 置顶 / 移动到文件夹 / 删除
 */
import { ref, computed, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button, Menu, Dialog, Icon, useToast, type MenuItemOption } from './basic'
import type { ConversationMeta, SearchHit } from '../types'

const props = defineProps<{
  /** 当前选中的会话 id，用于列表高亮 */
  activeId?: string | null
}>()

const emit = defineEmits<{
  (e: 'select-conversation', id: string | null): void
}>()

const { toast } = useToast()

// ---------- 会话列表 ----------
const conversations = ref<ConversationMeta[]>([])
const loading = ref(false)

const pinnedConversations = computed(() => conversations.value.filter((c) => c.pinned))
const regularConversations = computed(() => conversations.value.filter((c) => !c.pinned))
const pinnedCollapsed = ref(false)

// ---------- 文件夹（localStorage 持久化） ----------
interface ConvFolder {
  id: string
  name: string
  created_at: number
}
const FOLDERS_KEY = 'effisuite_conv_folders'
const FOLDER_MAP_KEY = 'effisuite_conv_folder_map'

const folders = ref<ConvFolder[]>([])
/** conversation_id → folder_id */
const convFolderMap = ref<Record<string, string>>({})
/** 当前筛选的文件夹 id（null = 全部会话） */
const activeFolderId = ref<string | null>(null)

function loadFolders() {
  try {
    folders.value = JSON.parse(localStorage.getItem(FOLDERS_KEY) || '[]')
    convFolderMap.value = JSON.parse(localStorage.getItem(FOLDER_MAP_KEY) || '{}')
  } catch {
    folders.value = []
    convFolderMap.value = {}
  }
}

function saveFolders() {
  localStorage.setItem(FOLDERS_KEY, JSON.stringify(folders.value))
  localStorage.setItem(FOLDER_MAP_KEY, JSON.stringify(convFolderMap.value))
}

function folderConvCount(folderId: string): number {
  return Object.values(convFolderMap.value).filter((id) => id === folderId).length
}

/** 当前文件夹筛选后的普通会话 */
const filteredConversations = computed(() => {
  if (!activeFolderId.value) return regularConversations.value
  return regularConversations.value.filter(
    (c) => convFolderMap.value[c.id] === activeFolderId.value,
  )
})

// 文件夹新建/改名对话框
const folderDialogVisible = ref(false)
const folderDialogMode = ref<'create' | 'rename'>('create')
const folderDialogTarget = ref<ConvFolder | null>(null)
const folderDialogName = ref('')

function openCreateFolder() {
  folderDialogMode.value = 'create'
  folderDialogTarget.value = null
  folderDialogName.value = ''
  folderDialogVisible.value = true
}

function openRenameFolder(f: ConvFolder) {
  folderDialogMode.value = 'rename'
  folderDialogTarget.value = f
  folderDialogName.value = f.name
  folderDialogVisible.value = true
}

function confirmFolderDialog() {
  const name = folderDialogName.value.trim()
  if (!name) {
    toast({ content: '文件夹名称不能为空', type: 'warn' })
    return
  }
  if (folderDialogMode.value === 'create') {
    folders.value.push({ id: `folder_${Date.now()}`, name, created_at: Date.now() })
    toast({ content: '文件夹已创建', type: 'success' })
  } else if (folderDialogTarget.value) {
    folderDialogTarget.value.name = name
    toast({ content: '文件夹已重命名', type: 'success' })
  }
  saveFolders()
  folderDialogVisible.value = false
}

function removeFolder(f: ConvFolder) {
  folders.value = folders.value.filter((x) => x.id !== f.id)
  for (const k of Object.keys(convFolderMap.value)) {
    if (convFolderMap.value[k] === f.id) delete convFolderMap.value[k]
  }
  if (activeFolderId.value === f.id) activeFolderId.value = null
  saveFolders()
  toast({ content: '文件夹已删除', type: 'success' })
}

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

// ---------- 会话右键 / 长按菜单 ----------
const menuVisible = ref(false)
const menuPosition = ref<{ x: number; y: number } | null>(null)
const menuTargetConv = ref<ConversationMeta | null>(null)

let longPressTimer: number | null = null

function onItemPointerDown(e: PointerEvent, conv: ConversationMeta) {
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
  const currentFolder = convFolderMap.value[c.id]
  return [
    { key: 'rename', label: '改名', icon: 'edit' },
    { key: 'pin', label: c.pinned ? '取消置顶' : '置顶', icon: c.pinned ? 'pin-filled' : 'pin' },
    {
      key: 'move-folder',
      label: '移动到文件夹',
      icon: 'folder',
      children: [
        ...folders.value.map((f) => ({
          key: `folder:${f.id}`,
          label: f.name,
          selected: currentFolder === f.id,
        })),
        { key: 'folder:none', label: '无文件夹', divided: folders.value.length > 0, selected: !currentFolder },
      ],
    },
    { key: 'delete', label: '删除', icon: 'delete', danger: true, divided: true },
  ]
})

function onMenuSelect(item: MenuItemOption) {
  const conv = menuTargetConv.value
  menuTargetConv.value = null
  if (!conv) return

  // 移动到文件夹
  if (item.key.startsWith('folder:')) {
    const folderId = item.key === 'folder:none' ? '' : item.key.slice('folder:'.length)
    if (folderId) convFolderMap.value[conv.id] = folderId
    else delete convFolderMap.value[conv.id]
    saveFolders()
    toast({ content: folderId ? '已移动到文件夹' : '已移出文件夹', type: 'success' })
    return
  }

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

// ---------- 文件夹右键 / 长按菜单 ----------
const folderMenuVisible = ref(false)
const folderMenuPosition = ref<{ x: number; y: number } | null>(null)
const folderMenuTarget = ref<ConvFolder | null>(null)

function onFolderContextMenu(e: MouseEvent, f: ConvFolder) {
  e.preventDefault()
  folderMenuPosition.value = { x: e.clientX, y: e.clientY }
  folderMenuTarget.value = f
  folderMenuVisible.value = true
}

const folderMenuItems: MenuItemOption[] = [
  { key: 'rename', label: '改名', icon: 'edit' },
  { key: 'delete', label: '删除', icon: 'delete', danger: true, divided: true },
]

function onFolderMenuSelect(item: MenuItemOption) {
  const f = folderMenuTarget.value
  folderMenuTarget.value = null
  if (!f) return
  if (item.key === 'rename') openRenameFolder(f)
  else if (item.key === 'delete') removeFolder(f)
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
    delete convFolderMap.value[id]
    saveFolders()
    await refresh()
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
  if (t) return t.length > 22 ? t.slice(0, 22) + '…' : t
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

onMounted(() => {
  loadFolders()
  refresh()
})

defineExpose({ refresh })
</script>

<template>
  <aside class="history-rail">
    <!-- 顶部：新建聊天 -->
    <div class="hr-top">
      <Button variant="primary" block @click="onNewChat">
        <template #icon>＋</template>
        新建聊天
      </Button>
    </div>

    <!-- 搜索框 -->
    <div class="hr-search">
      <span class="hr-search-icon"><Icon name="search" :size="16" /></span>
      <input
        v-model="searchQuery"
        type="text"
        class="hr-search-input"
        placeholder="搜索会话..."
      />
    </div>

    <!-- 列表主体 -->
    <div class="hr-body">
      <!-- 搜索结果 -->
      <template v-if="searchQuery.trim() && searchResults.length > 0">
        <div class="hr-section-title">搜索结果</div>
        <div
          v-for="hit in searchResults"
          :key="hit.conversation_id + hit.message_id"
          class="hr-item"
          :class="{ active: hit.conversation_id === props.activeId }"
          @click="emit('select-conversation', hit.conversation_id)"
        >
          <div class="hr-item-title">{{ hit.conversation_title || '新对话' }}</div>
          <div class="hr-item-snippet">{{ hit.snippet }}</div>
        </div>
      </template>

      <!-- 搜索无结果 -->
      <div v-else-if="searchQuery.trim() && searchResults.length === 0" class="hr-empty">
        未找到相关会话
      </div>

      <!-- 正常列表 -->
      <template v-else>
        <!-- 置顶分组 -->
        <template v-if="pinnedConversations.length > 0">
          <div
            class="hr-section-title collapsible"
            @click="pinnedCollapsed = !pinnedCollapsed"
          >
            <span class="hr-section-arrow">
              <Icon :name="pinnedCollapsed ? 'chevron-right' : 'chevron-down'" :size="13" />
            </span>
            <span class="hr-section-label">置顶</span>
            <span class="hr-section-count">{{ pinnedConversations.length }}</span>
          </div>
          <template v-if="!pinnedCollapsed">
            <div
              v-for="c in pinnedConversations"
              :key="c.id"
              class="hr-item"
              :class="{ active: c.id === props.activeId }"
              @click="emit('select-conversation', c.id)"
              @pointerdown="onItemPointerDown($event, c)"
              @pointerup="onItemPointerUp"
              @pointerleave="onItemPointerUp"
              @pointercancel="onItemPointerUp"
              @contextmenu="onItemContextMenu($event, c)"
            >
              <span class="hr-item-pin"><Icon name="pin" :size="13" /></span>
              <div class="hr-item-main">
                <div class="hr-item-title">{{ displayTitle(c) }}</div>
                <div class="hr-item-meta">
                  {{ c.message_count }} 条 · {{ formatRelativeTime(c.updated_at) }}
                </div>
              </div>
            </div>
          </template>
        </template>

        <!-- 文件夹分组 -->
        <div class="hr-folder-head">
          <span class="hr-section-label">文件夹</span>
          <button
            type="button"
            class="hr-folder-add"
            title="新建文件夹"
            aria-label="新建文件夹"
            @click="openCreateFolder"
          >
            <Icon name="plus" :size="14" />
          </button>
        </div>

        <div
          class="hr-folder-item"
          :class="{ active: activeFolderId === null }"
          @click="activeFolderId = null"
        >
          <span class="hr-folder-icon"><Icon name="chat" :size="14" /></span>
          <span class="hr-folder-name">全部会话</span>
          <span class="hr-folder-count">{{ regularConversations.length }}</span>
        </div>

        <div
          v-for="f in folders"
          :key="f.id"
          class="hr-folder-item"
          :class="{ active: activeFolderId === f.id }"
          @click="activeFolderId = activeFolderId === f.id ? null : f.id"
          @contextmenu="onFolderContextMenu($event, f)"
        >
          <span class="hr-folder-icon"><Icon name="folder" :size="14" /></span>
          <span class="hr-folder-name">{{ f.name }}</span>
          <span class="hr-folder-count">{{ folderConvCount(f.id) }}</span>
        </div>

        <div v-if="folders.length === 0" class="hr-folder-hint">
          用文件夹给会话分类，右键会话即可移动
        </div>

        <!-- 会话列表 -->
        <template v-if="activeFolderId">
          <div class="hr-section-title folder-filter-title">
            <span class="hr-folder-icon"><Icon name="folder" :size="13" /></span>
            <span class="hr-section-label">{{ folders.find((f) => f.id === activeFolderId)?.name }}</span>
            <button
              type="button"
              class="hr-filter-clear"
              title="清除筛选"
              @click="activeFolderId = null"
            >
              <Icon name="close" :size="12" />
            </button>
          </div>
        </template>

        <div
          v-for="c in filteredConversations"
          :key="c.id"
          class="hr-item"
          :class="{ active: c.id === props.activeId }"
          @click="emit('select-conversation', c.id)"
          @pointerdown="onItemPointerDown($event, c)"
          @pointerup="onItemPointerUp"
          @pointerleave="onItemPointerUp"
          @pointercancel="onItemPointerUp"
          @contextmenu="onItemContextMenu($event, c)"
        >
          <div class="hr-item-main">
            <div class="hr-item-title">{{ displayTitle(c) }}</div>
            <div class="hr-item-meta">
              {{ c.message_count }} 条 · {{ formatRelativeTime(c.updated_at) }}
            </div>
          </div>
        </div>

        <!-- 空状态 -->
        <div v-if="conversations.length === 0 && !loading" class="hr-empty">
          暂无会话，点击上方新建
        </div>
        <div
          v-else-if="activeFolderId && filteredConversations.length === 0"
          class="hr-empty"
        >
          该文件夹暂无会话
        </div>
      </template>
    </div>

    <!-- 会话上下文菜单 -->
    <Menu
      v-model:visible="menuVisible"
      :items="itemMenuItems"
      :position="menuPosition"
      @select="onMenuSelect"
    />

    <!-- 文件夹上下文菜单 -->
    <Menu
      v-model:visible="folderMenuVisible"
      :items="folderMenuItems"
      :position="folderMenuPosition"
      @select="onFolderMenuSelect"
    />

    <!-- 文件夹新建/改名对话框 -->
    <Dialog
      v-model:visible="folderDialogVisible"
      :title="folderDialogMode === 'create' ? '新建文件夹' : '重命名文件夹'"
      confirm-text="保存"
      cancel-text="取消"
      @confirm="confirmFolderDialog"
    >
      <input
        v-model="folderDialogName"
        type="text"
        class="hr-input"
        placeholder="输入文件夹名称"
        @keyup.enter="confirmFolderDialog"
      />
    </Dialog>

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
        class="hr-input"
        placeholder="输入新标题（留空使用首条消息摘要）"
        @keyup.enter="confirmRename"
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
      <div class="hr-dialog-delete">确定删除该会话？此操作不可撤销。</div>
    </Dialog>
  </aside>
</template>

<style scoped>
.history-rail {
  display: flex;
  flex-direction: column;
  width: 248px;
  flex-shrink: 0;
  background: var(--bg-2);
  border-right: 1px solid var(--border);
  overflow: hidden;
  user-select: none;
}

/* 顶部新建聊天 */
.hr-top {
  padding: 12px 12px 8px;
  flex-shrink: 0;
}

/* 搜索框 */
.hr-search {
  display: flex;
  align-items: center;
  gap: 7px;
  margin: 0 12px 8px;
  padding: 0 10px;
  height: 32px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  flex-shrink: 0;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.hr-search:focus-within {
  border-color: var(--primary);
}

.hr-search-icon {
  display: inline-flex;
  color: var(--muted);
  flex-shrink: 0;
}

.hr-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  outline: none;
  font-family: inherit;
  font-size: 13px;
  color: var(--text);
}

.hr-search-input::placeholder {
  color: var(--muted);
}

/* 主体滚动区 */
.hr-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  padding: 4px 8px 12px;
}

.hr-section-title {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 10px 8px 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
}

.hr-section-title.collapsible {
  cursor: pointer;
}

.hr-section-title.collapsible:hover {
  color: var(--text);
}

.hr-section-arrow {
  display: inline-flex;
  color: var(--muted);
}

.hr-section-count {
  font-size: 11px;
  font-weight: 400;
  color: var(--muted);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 0 6px;
  margin-left: 4px;
}

.hr-section-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
}

/* 文件夹 */
.hr-folder-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 8px 6px;
}

.hr-folder-add {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.hr-folder-add:hover {
  background: var(--card);
  color: var(--primary);
}

.hr-folder-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.hr-folder-item:hover {
  background: var(--card);
}

.hr-folder-item.active {
  background: rgba(74, 126, 255, 0.12);
}

.hr-folder-item.active .hr-folder-name {
  color: var(--primary);
  font-weight: 500;
}

.hr-folder-icon {
  display: inline-flex;
  color: var(--muted);
  flex-shrink: 0;
}

.hr-folder-item.active .hr-folder-icon {
  color: var(--primary);
}

.hr-folder-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hr-folder-count {
  font-size: 11px;
  color: var(--muted);
  flex-shrink: 0;
}

.hr-folder-hint {
  padding: 4px 10px 8px;
  font-size: 11px;
  color: var(--muted);
  line-height: 1.5;
}

.folder-filter-title {
  gap: 6px;
}

.hr-filter-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast), color var(--duration-fast);
}

.hr-filter-clear:hover {
  background: var(--card);
  color: var(--text);
}

/* 会话项 */
.hr-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 9px 10px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.hr-item:hover {
  background: var(--card);
}

.hr-item.active {
  background: rgba(74, 126, 255, 0.12);
}

.hr-item.active .hr-item-title {
  color: var(--primary);
}

.hr-item-main {
  flex: 1;
  min-width: 0;
}

.hr-item-pin {
  display: inline-flex;
  margin-top: 2px;
  color: var(--warn);
  flex-shrink: 0;
}

.hr-item-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hr-item-meta {
  font-size: 11px;
  color: var(--muted);
  margin-top: 2px;
}

.hr-item-snippet {
  font-size: 12px;
  color: var(--muted);
  margin-top: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.hr-empty {
  padding: 24px 12px;
  text-align: center;
  font-size: 12px;
  color: var(--muted);
}

/* 输入框（对话框内） */
.hr-input {
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

.hr-input:focus {
  border-color: var(--primary);
}

.hr-dialog-delete {
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  padding: 4px 0;
}

/* 滚动条 */
.hr-body::-webkit-scrollbar {
  width: 6px;
}

.hr-body::-webkit-scrollbar-thumb {
  background: var(--border);
  border-radius: 3px;
}

.hr-body::-webkit-scrollbar-thumb:hover {
  background: var(--muted);
}
</style>
