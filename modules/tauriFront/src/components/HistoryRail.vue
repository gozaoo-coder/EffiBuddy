<script setup lang="ts">
/**
 * HistoryRail 第二栏：历史记录栏
 *
 * 从上到下：
 *  1. 新建聊天按钮
 *  2. 搜索框
 *  3. 置顶（可折叠）
 *  4. 文件夹（全部 / 未分类 / 用户自定义，localStorage 持久化）
 *  5. 会话列表（可按文件夹筛选，支持多选 + 自动归类）
 *
 * 会话右键/长按菜单：改名 / 置顶 / 移动到文件夹 / 自动归类 / 删除
 * 会话项内嵌按钮：更多（打开菜单） / 自动归类（AI 生成标题 + 归类）
 *
 * 模块拆分（减少上帝文件）：
 * - 文件夹状态 → useConversationFolders composable
 * - 多选状态 → useHistorySelection composable
 * - 单条会话项渲染 → HistoryItem 组件
 * - 批量操作浮动栏 → HistorySelectionBar 组件
 */
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Button, Menu, Dialog, Icon, useToast, type MenuItemOption } from './basic'
import HistoryItem from './HistoryItem.vue'
import HistorySelectionBar from './HistorySelectionBar.vue'
import { useConversationFolders, UNCLASSIFIED } from '../composables/useConversationFolders'
import { useHistorySelection } from '../composables/useHistorySelection'
import type { ConversationMeta, SearchHit, AutoClassifyResult, BatchDeleteResult, PoolEntry, PoolStatus } from '../types'

const props = defineProps<{
  /** 当前选中的会话 id，用于列表高亮 */
  activeId?: string | null
}>()

const emit = defineEmits<{
  // title 可选：打开已有会话时携带标题，供多页签系统直接设置页签标题，
  // 避免 App.vue 再发起一次后端请求。新建聊天（id=null）不传 title。
  (e: 'select-conversation', id: string | null, title?: string | null): void
}>()

const { toast } = useToast()

// ---------- 会话列表 ----------
const conversations = ref<ConversationMeta[]>([])
const loading = ref(false)

const pinnedConversations = computed(() => conversations.value.filter((c) => c.pinned))
const regularConversations = computed(() => conversations.value.filter((c) => !c.pinned))
const pinnedCollapsed = ref(false)

// ---------- 交流池运行状态（会话列表展示各会话 agent 工作状态） ----------
const poolEntries = ref<PoolEntry[]>([])

/** 每个会话聚合后的交流池状态：in_progress > waiting；已完成不展示 */
const poolStatusByConv = computed<Map<string, PoolStatus>>(() => {
  const m = new Map<string, PoolStatus>()
  for (const e of poolEntries.value) {
    if (e.status === 'completed') continue
    const cur = m.get(e.conversation_id)
    if (!cur || (cur === 'waiting' && e.status === 'in_progress')) {
      m.set(e.conversation_id, e.status)
    }
  }
  return m
})

/** 会话的活跃交流池状态（无条目 / 全部完成 → null，不展示 badge） */
function poolStatusFor(id: string): PoolStatus | null {
  return poolStatusByConv.value.get(id) ?? null
}

/** 会话在交流池登记的最新任务描述（badge hover 提示） */
function poolTaskFor(id: string): string {
  const e = poolEntries.value.find(
    (x) => x.conversation_id === id && x.status !== 'completed',
  )
  return e ? e.task : ''
}

async function loadPool() {
  try {
    poolEntries.value = await invoke<PoolEntry[]>('list_pool')
  } catch {
    poolEntries.value = []
  }
}

let unlistenPool: (() => void) | null = null
// ---------- 文件夹（composable） ----------
const {
  folders,
  convFolderMap,
  activeFolderId,
  loadFolders,
  saveFolders,
  folderConvCount,
  unclassifiedCount,
  getConvFolder,
  createFolder,
  renameFolder,
  removeFolder,
  moveConvToFolder,
  batchMoveConvToFolder,
  findFolderByName,
  folderNames,
} = useConversationFolders()

/** 当前文件夹筛选后的普通会话 */
const filteredConversations = computed(() => {
  if (activeFolderId.value === null) return regularConversations.value
  if (activeFolderId.value === UNCLASSIFIED) {
    return regularConversations.value.filter((c) => !getConvFolder(c.id))
  }
  return regularConversations.value.filter((c) => getConvFolder(c.id) === activeFolderId.value)
})

/** 当前视图所有会话 id（用于多选全选） */
const currentViewIds = computed(() => filteredConversations.value.map((c) => c.id))

// ---------- 多选（composable） ----------
const {
  selectionMode,
  selectedCount,
  enterSelectionMode,
  exitSelectionMode,
  toggleSelected,
  isSelected,
  selectAll,
  selectNone,
  getSelectedArray,
} = useHistorySelection()

const allSelected = computed(
  () => selectedCount.value === currentViewIds.value.length && selectedCount.value > 0,
)

// ---------- 自动归类状态追踪 ----------
/** 正在归类的会话 id 集合（单条 + 批量共用） */
const classifyingIds = ref<Set<string>>(new Set())
const batchClassifying = ref(false)

function setClassifying(id: string, value: boolean) {
  const next = new Set(classifyingIds.value)
  if (value) next.add(id)
  else next.delete(id)
  classifyingIds.value = next
}

// ---------- 文件夹新建/改名对话框 ----------
const folderDialogVisible = ref(false)
const folderDialogMode = ref<'create' | 'rename'>('create')
const folderDialogTarget = ref<{ id: string; name: string; created_at: number } | null>(null)
const folderDialogName = ref('')

function openCreateFolder() {
  folderDialogMode.value = 'create'
  folderDialogTarget.value = null
  folderDialogName.value = ''
  folderDialogVisible.value = true
}

function openRenameFolder(f: { id: string; name: string; created_at: number }) {
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
    createFolder(name)
    toast({ content: '文件夹已创建', type: 'success' })
  } else if (folderDialogTarget.value) {
    renameFolder(folderDialogTarget.value, name)
    toast({ content: '文件夹已重命名', type: 'success' })
  }
  folderDialogVisible.value = false
}

function removeFolderAndToast(f: { id: string }) {
  removeFolder(f.id)
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
  if (selectionMode.value) return
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
  if (selectionMode.value) return
  e.preventDefault()
  openItemMenu(conv, e.clientX, e.clientY)
}

/** more 按钮点击：在按钮下方打开菜单 */
function onItemMore(ev: MouseEvent, conv: ConversationMeta) {
  const target = ev.currentTarget as HTMLElement
  const rect = target.getBoundingClientRect()
  openItemMenu(conv, rect.left, rect.bottom + 4)
}

function openItemMenu(conv: ConversationMeta, x: number, y: number) {
  menuPosition.value = { x, y }
  menuTargetConv.value = conv
  menuVisible.value = true
}

const itemMenuItems = computed<MenuItemOption[]>(() => {
  const c = menuTargetConv.value
  if (!c) return []
  const currentFolder = getConvFolder(c.id)
  return [
    { key: 'rename', label: '改名', icon: 'edit' },
    { key: 'auto-classify', label: '自动归类', icon: 'sparkles' },
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
    const folderId = item.key === 'folder:none' ? null : item.key.slice('folder:'.length)
    moveConvToFolder(conv.id, folderId)
    toast({ content: folderId ? '已移动到文件夹' : '已移出文件夹', type: 'success' })
    return
  }

  switch (item.key) {
    case 'rename':
      startRename(conv)
      break
    case 'auto-classify':
      doAutoClassify(conv)
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

// ---------- 文件夹右键菜单 ----------
const folderMenuVisible = ref(false)
const folderMenuPosition = ref<{ x: number; y: number } | null>(null)
const folderMenuTarget = ref<{ id: string; name: string; created_at: number } | null>(null)

function onFolderContextMenu(e: MouseEvent, f: { id: string; name: string; created_at: number }) {
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
  else if (item.key === 'delete') removeFolderAndToast(f)
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

// ---------- 自动归类（单条） ----------
async function doAutoClassify(conv: ConversationMeta) {
  setClassifying(conv.id, true)
  try {
    const result = await invoke<AutoClassifyResult>('auto_classify_conversation', {
      conversationId: conv.id,
      existingFolders: folderNames(),
    })
    // 标题已由后端持久化；这里只需更新文件夹映射
    if (result.folder) {
      const folder = findFolderByName(result.folder)
      if (folder) {
        moveConvToFolder(conv.id, folder.id)
      }
    }
    await refresh()
    toast({
      content: result.folder ? `已归类到「${result.folder}」` : '已设置标题',
      type: 'success',
    })
  } catch (e) {
    toast({ content: `归类失败：${e}`, type: 'error' })
  } finally {
    setClassifying(conv.id, false)
  }
}

// ---------- 批量自动归类 ----------
async function onBatchAutoClassify() {
  const ids = getSelectedArray()
  if (ids.length === 0) return
  batchClassifying.value = true
  let success = 0
  let fail = 0
  for (const id of ids) {
    setClassifying(id, true)
    try {
      const result = await invoke<AutoClassifyResult>('auto_classify_conversation', {
        conversationId: id,
        existingFolders: folderNames(),
      })
      if (result.folder) {
        const folder = findFolderByName(result.folder)
        if (folder) {
          moveConvToFolder(id, folder.id)
        }
      }
      success++
    } catch {
      fail++
    } finally {
      setClassifying(id, false)
    }
  }
  batchClassifying.value = false
  await refresh()
  if (fail === 0) {
    toast({ content: `已归类 ${success} 条会话`, type: 'success' })
  } else {
    toast({ content: `成功 ${success} 条，失败 ${fail} 条`, type: 'warn' })
  }
  exitSelectionMode()
}

// ---------- 批量移动 ----------
function onBatchMove(folderId: string | null) {
  const ids = getSelectedArray()
  if (ids.length === 0) return
  batchMoveConvToFolder(ids, folderId)
  toast({ content: folderId ? `已移动 ${ids.length} 条到文件夹` : `已移出 ${ids.length} 条`, type: 'success' })
  exitSelectionMode()
}

// ---------- 删除 ----------
const deleteDialogVisible = ref(false)
const deleteTargetId = ref<string | null>(null)

async function confirmDelete() {
  const id = deleteTargetId.value
  if (!id) return
  try {
    await invoke('delete_conversation', { id })
    moveConvToFolder(id, null)
    await refresh()
    if (id === props.activeId) {
      const nextConv = conversations.value[0] ?? null
      emit('select-conversation', nextConv?.id ?? null, nextConv?.title ?? null)
    }
    toast({ content: '会话已删除', type: 'success' })
  } catch (e) {
    toast({ content: `删除会话失败：${e}`, type: 'error' })
  } finally {
    deleteTargetId.value = null
  }
}

// ---------- 批量删除 ----------
const batchDeleteDialogVisible = ref(false)
const batchDeleting = ref(false)

/** 多选模式下点击删除按钮：打开确认对话框 */
function onBatchDeleteClick() {
  if (selectedCount.value === 0 || batchDeleting.value) return
  batchDeleteDialogVisible.value = true
}

/**
 * 批量删除确认：
 * 1. 调用 delete_conversations 一次性删除所有选中会话（后端单次锁清理交流池）
 * 2. 清理本地文件夹映射（批量移出）
 * 3. 刷新会话列表
 * 4. 若当前激活会话在删除集合中，切换到第一条剩余会话
 * 5. 根据成功/失败数提示；最后退出多选模式（驱动选中栏滑出动画）
 */
async function confirmBatchDelete() {
  const ids = getSelectedArray()
  if (ids.length === 0) {
    batchDeleteDialogVisible.value = false
    return
  }
  batchDeleting.value = true
  try {
    const result = await invoke<BatchDeleteResult>('delete_conversations', { ids })
    // 仅清理成功删除的会话的文件夹映射；失败的会话仍保留其文件夹归属
    const failedSet = new Set(result.failed)
    const successIds = ids.filter((id) => !failedSet.has(id))
    if (successIds.length > 0) {
      batchMoveConvToFolder(successIds, null)
    }
    await refresh()

    // 当前激活会话已被删除（不在列表中）→ 切换到第一条剩余会话
    // 若激活会话删除失败仍存在，则保持不变
    if (props.activeId && !conversations.value.some((c) => c.id === props.activeId)) {
      const nextConv = conversations.value[0] ?? null
      emit('select-conversation', nextConv?.id ?? null, nextConv?.title ?? null)
    }

    // 结果提示
    const failedCount = result.failed.length
    if (failedCount === 0) {
      toast({ content: `已删除 ${result.success} 条会话`, type: 'success' })
    } else if (result.success === 0) {
      toast({ content: `删除失败（${failedCount} 条）`, type: 'error' })
    } else {
      toast({
        content: `成功删除 ${result.success} 条，失败 ${failedCount} 条`,
        type: 'warn',
      })
    }

    // 退出多选模式（HistorySelectionBar 触发滑出动画）
    exitSelectionMode()
  } catch (e) {
    toast({ content: `批量删除失败：${e}`, type: 'error' })
  } finally {
    batchDeleting.value = false
    batchDeleteDialogVisible.value = false
  }
}

// ---------- 新建聊天 ----------
function onNewChat() {
  emit('select-conversation', null)
}

// ---------- 多选操作 ----------
function onEnterSelection() {
  enterSelectionMode()
}

function onExitSelection() {
  exitSelectionMode()
}

function onSelectAllToggle() {
  if (allSelected.value) {
    selectNone()
  } else {
    selectAll(currentViewIds.value)
  }
}

// ---------- 工具函数 ----------
function displayTitle(conv: ConversationMeta): string {
  const t = conv.title?.trim()
  if (t) return t.length > 18 ? t.slice(0, 18) + '…' : t
  return '新对话'
}

/** 当前视图标题 */
const currentViewTitle = computed(() => {
  if (activeFolderId.value === null) return '全部会话'
  if (activeFolderId.value === UNCLASSIFIED) return '未分类'
  return folders.value.find((f) => f.id === activeFolderId.value)?.name ?? '会话'
})

/** 全部会话 id（用于未分类计数） */
const allConvIds = computed(() => conversations.value.filter(c => !c.pinned).map(c => c.id))

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
  loadPool()
  // 交流池更新（pool_report / pool_at / pool_reply 后）→ 刷新会话列表运行状态
  listen<{ conversation_id: string }>('agent-pool-updated', () => {
    loadPool()
  }).then((fn) => {
    unlistenPool = fn
  })
})

onUnmounted(() => {
  unlistenPool?.()
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

    <!-- 顶部固定区与滚动主体的分割线 -->
    <div class="hr-divider hr-divider--top" />

    <!-- 列表主体 -->
    <div class="hr-body" :class="{ 'has-sel-bar': selectionMode }">
      <!-- 搜索结果 -->
      <template v-if="searchQuery.trim() && searchResults.length > 0">
        <div class="hr-section-title">搜索结果</div>
        <div
          v-for="hit in searchResults"
          :key="hit.conversation_id + hit.message_id"
          class="hr-item hr-item--search"
          :class="{ active: hit.conversation_id === props.activeId }"
          @click="emit('select-conversation', hit.conversation_id, hit.conversation_title)"
        >
          <div class="hr-item-main">
            <div class="hr-item-title">{{ hit.conversation_title || '新对话' }}</div>
            <div class="hr-item-snippet">{{ hit.snippet }}</div>
          </div>
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
            <HistoryItem
              v-for="c in pinnedConversations"
              :key="c.id"
              :conv="c"
              :active="c.id === props.activeId"
              :display-title="displayTitle(c)"
              :selection-mode="selectionMode"
              :selected="isSelected(c.id)"
              :classifying="classifyingIds.has(c.id)"
              :show-pin="true"
              :pool-status="poolStatusFor(c.id)"
              :pool-task="poolTaskFor(c.id)"
              @click="emit('select-conversation', c.id, c.title)"
              @contextmenu="onItemContextMenu($event, c)"
              @pointerdown="onItemPointerDown($event, c)"
              @pointerup="onItemPointerUp"
              @pointerleave="onItemPointerUp"
              @pointercancel="onItemPointerUp"
              @toggle-select="toggleSelected(c.id)"
              @more="onItemMore($event, c)"
              @auto-classify="doAutoClassify(c)"
            />
          </template>
        </template>

        <!-- 置顶与文件夹分组的分割线（仅有置顶时显示） -->
        <div v-if="pinnedConversations.length > 0" class="hr-divider" />

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
          class="hr-folder-item"
          :class="{ active: activeFolderId === UNCLASSIFIED }"
          @click="activeFolderId = activeFolderId === UNCLASSIFIED ? null : UNCLASSIFIED"
        >
          <span class="hr-folder-icon"><Icon name="folder" :size="14" /></span>
          <span class="hr-folder-name">未分类</span>
          <span class="hr-folder-count">{{ unclassifiedCount(allConvIds) }}</span>
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

        <!-- 文件夹分组与会话列表的分割线 -->
        <div class="hr-divider" />

        <!-- 会话列表标题栏 -->
        <div class="hr-list-header">
          <span class="hr-folder-icon"><Icon name="folder" :size="13" /></span>
          <span class="hr-list-title">{{ currentViewTitle }}</span>
          <span class="hr-list-count">{{ filteredConversations.length }}</span>
          <button
            v-if="activeFolderId !== null"
            type="button"
            class="hr-list-clear"
            title="清除筛选"
            @click="activeFolderId = null"
          >
            <Icon name="close" :size="12" />
          </button>
          <button
            v-if="!selectionMode && filteredConversations.length > 0"
            type="button"
            class="hr-list-select-btn"
            title="多选"
            @click="onEnterSelection"
          >
            <Icon name="check" :size="13" />
            <span>多选</span>
          </button>
        </div>

        <!-- 会话列表 -->
        <HistoryItem
          v-for="c in filteredConversations"
          :key="c.id"
          :conv="c"
          :active="c.id === props.activeId"
          :display-title="displayTitle(c)"
          :selection-mode="selectionMode"
          :selected="isSelected(c.id)"
          :classifying="classifyingIds.has(c.id)"
            :show-pin="false"
            :pool-status="poolStatusFor(c.id)"
            :pool-task="poolTaskFor(c.id)"
          @click="emit('select-conversation', c.id, c.title)"
          @contextmenu="onItemContextMenu($event, c)"
          @pointerdown="onItemPointerDown($event, c)"
          @pointerup="onItemPointerUp"
          @pointerleave="onItemPointerUp"
          @pointercancel="onItemPointerUp"
          @toggle-select="toggleSelected(c.id)"
          @more="onItemMore($event, c)"
          @auto-classify="doAutoClassify(c)"
        />

        <!-- 空状态 -->
        <div v-if="conversations.length === 0 && !loading" class="hr-empty">
          暂无会话，点击上方新建
        </div>
        <div
          v-else-if="filteredConversations.length === 0"
          class="hr-empty"
        >
          {{ activeFolderId === UNCLASSIFIED ? '没有未分类的会话' : '该文件夹暂无会话' }}
        </div>
      </template>
    </div>

    <!-- 多选浮动操作栏 -->
    <HistorySelectionBar
      v-if="selectionMode"
      :selected-count="selectedCount"
      :total-count="currentViewIds.length"
      :all-selected="allSelected"
      :folders="folders"
      :batch-classifying="batchClassifying"
      :batch-deleting="batchDeleting"
      @select-all="onSelectAllToggle"
      @clear="selectNone"
      @batch-move="onBatchMove"
      @batch-auto-classify="onBatchAutoClassify"
      @batch-delete="onBatchDeleteClick"
      @cancel="onExitSelection"
    />

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

    <!-- 删除确认对话框（单条） -->
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

    <!-- 批量删除确认对话框（多选模式） -->
    <Dialog
      v-model:visible="batchDeleteDialogVisible"
      title="批量删除会话"
      danger
      confirm-text="删除"
      cancel-text="取消"
      :close-on-click-overlay="false"
      @confirm="confirmBatchDelete"
    >
      <div class="hr-dialog-delete">
        确定删除选中的 <strong>{{ selectedCount }}</strong> 条会话？此操作不可撤销。
      </div>
    </Dialog>
  </aside>
</template>

<style scoped>
.history-rail {
  display: flex;
  flex-direction: column;
  width: 220px;
  flex-shrink: 0;
  background: var(--bg-rail-2);
  border-right: 1px solid var(--border-strong);
  overflow: hidden;
  user-select: none;
}

/* 顶部新建聊天：大边界留白，视觉轻松 */
.hr-top {
  padding: 14px 12px 8px;
  flex-shrink: 0;
}

/* 搜索框 */
.hr-search {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 12px 10px;
  padding: 0 10px;
  height: 30px;
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
  font-size: 12px;
  color: var(--text);
}

.hr-search-input::placeholder {
  color: var(--muted);
}

/* 分割线：统一细线，配合 opacity 过渡以适配折叠/搜索模式切换 */
.hr-divider {
  height: 1px;
  background: var(--border);
  flex-shrink: 0;
  opacity: 1;
  transition: opacity var(--duration-fast) var(--ease-standard);
}

/* 顶部固定区与滚动主体之间的分割线（位于 .hr-body 外，左右对齐搜索框 12px） */
.hr-divider--top {
  margin: 0 12px;
}

/* 主体内部分割线（位于 .hr-body 内，body 已有 10px 横向 padding，故 2px 即可对齐 12px） */
.hr-body > .hr-divider {
  margin: 6px 2px 4px;
}

/* 主体滚动区：大边界布局——外边距宽松，内部元素紧凑排列 */
.hr-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  padding: 8px 10px 14px;
  /* 多选栏出现时底部留出空间 */
  transition: padding-bottom var(--duration-fast) var(--ease-standard);
}

.hr-body.has-sel-bar {
  padding-bottom: 4px;
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
  font-size: 10px;
  font-weight: 400;
  color: var(--muted);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 0 5px;
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
  padding: 10px 8px 6px;
}

.hr-folder-add {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.hr-folder-add:hover {
  background: var(--hover, var(--card));
  color: var(--primary);
}

.hr-folder-item {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 6px 10px;
  margin: 1px 0;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.hr-folder-item:hover {
  background: var(--hover, var(--card));
}

.hr-folder-item.active {
  background: rgba(0, 0, 0, 0.08);
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
  font-size: 10px;
  color: var(--muted);
  flex-shrink: 0;
}

.hr-folder-hint {
  padding: 2px 8px 6px;
  font-size: 11px;
  color: var(--muted);
  line-height: 1.5;
}

/* 会话列表标题栏 */
.hr-list-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 8px 6px;
}

.hr-list-title {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hr-list-count {
  font-size: 10px;
  font-weight: 400;
  color: var(--muted);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  padding: 0 6px;
  flex-shrink: 0;
}

.hr-list-clear {
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
  flex-shrink: 0;
  transition: background var(--duration-fast), color var(--duration-fast);
}

.hr-list-clear:hover {
  background: var(--hover, var(--card));
  color: var(--text);
}

.hr-list-select-btn {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px 8px;
  border: none;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--muted);
  font-size: 11px;
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.hr-list-select-btn:hover {
  background: var(--hover, var(--card));
  color: var(--primary);
}

/* 搜索结果项（复用 hr-item 样式） */
.hr-item--search {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  padding: 7px 10px;
  margin: 1px 0;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard);
}

.hr-item--search:hover {
  background: var(--hover, var(--card));
}

.hr-item--search.active {
  background: rgba(0, 0, 0, 0.08);
}

.hr-item--search.active .hr-item-title {
  color: var(--primary);
}

.hr-item-main {
  flex: 1;
  min-width: 0;
}

.hr-item-title {
  font-size: 11px;
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hr-item-snippet {
  font-size: 11px;
  color: var(--muted);
  margin-top: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.hr-empty {
  padding: 16px 8px;
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
  border-radius: var(--radius-md);
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
