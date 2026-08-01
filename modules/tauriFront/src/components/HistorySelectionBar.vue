<script setup lang="ts">
/**
 * HistorySelectionBar —— 多选模式浮动操作栏
 *
 * 从 HistoryRail.vue 抽取的独立组件，职责：
 * - 显示已选数量 / 全选切换
 * - 批量移动到文件夹（内嵌 Menu 展示文件夹列表）
 * - 批量自动归类
 * - 退出多选模式
 *
 * 动画管线设计（全流程）：
 * - 进入：从底部滑入（translateY 100%→0）+ fade，300ms ease-out(3)
 * - 离开：向底部滑出（translateY 0→100%）+ fade，240ms ease-inOut(2)
 * - 中断处理：anime.js onComplete 驱动 Vue done()，过渡中被 v-if 移除也能平滑退出
 * - 父级 padding 同步过渡：选中栏出现时父级底部留出空间，避免遮挡最后一条 item
 */
import { ref, computed } from 'vue'
import { Icon, Menu, type MenuItemOption } from './basic'
import { useAnimeTransition } from '../composables/useAnimeTransition'
import type { ConvFolder } from '../composables/useConversationFolders'

const props = defineProps<{
  selectedCount: number
  /** 当前视图可选项总数 */
  totalCount: number
  /** 是否全选（selectedCount === totalCount 且 > 0） */
  allSelected: boolean
  /** 文件夹列表（批量移动用） */
  folders: ConvFolder[]
  /** 批量自动归类进行中 */
  batchClassifying: boolean
}>()

const emit = defineEmits<{
  (e: 'select-all'): void
  (e: 'clear'): void
  (e: 'batch-move', folderId: string | null): void
  (e: 'batch-auto-classify'): void
  (e: 'cancel'): void
}>()

// 进入/离开动画：从底部滑入 / 向底部滑出
const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    transform: ['translateY(100%)', 'translateY(0px)'],
    duration: 300,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    transform: ['translateY(0px)', 'translateY(100%)'],
    duration: 240,
    ease: 'inOut(2)',
  },
})

// 批量移动菜单
const moveMenuVisible = ref(false)
const moveMenuTrigger = ref<HTMLElement | null>(null)

const moveMenuItems = computed<MenuItemOption[]>(() => [
  ...props.folders.map((f) => ({
    key: `folder:${f.id}`,
    label: f.name,
    icon: 'folder',
  })),
  {
    key: 'folder:none',
    label: '移出文件夹',
    icon: 'close',
    divided: props.folders.length > 0,
  },
])

function onMoveClick() {
  moveMenuTrigger.value = moveBtnRef.value
  moveMenuVisible.value = true
}

const moveBtnRef = ref<HTMLElement | null>(null)

function onMoveMenuSelect(item: MenuItemOption) {
  const folderId = item.key === 'folder:none' ? null : item.key.slice('folder:'.length)
  emit('batch-move', folderId)
}

const canBatch = computed(() => props.selectedCount > 0 && !props.batchClassifying)
</script>

<template>
  <Transition :css="false" @enter="onEnter" @leave="onLeave">
    <div class="hr-sel-bar">
      <!-- 左侧：全选 + 计数 -->
      <button
        type="button"
        class="hr-sel-select-all"
        :disabled="totalCount === 0"
        @click="allSelected ? emit('clear') : emit('select-all')"
      >
        <span class="hr-sel-check" :class="{ checked: allSelected, partial: !allSelected && selectedCount > 0 }">
          <Icon v-if="allSelected" name="check" :size="12" />
        </span>
        <span class="hr-sel-count">{{ selectedCount }}/{{ totalCount }}</span>
      </button>

      <!-- 右侧：批量操作 -->
      <div class="hr-sel-actions">
        <button
          ref="moveBtnRef"
          type="button"
          class="hr-sel-btn"
          :disabled="!canBatch"
          @click="onMoveClick"
        >
          <Icon name="move" :size="14" />
          <span>移动</span>
        </button>
        <button
          type="button"
          class="hr-sel-btn hr-sel-btn--primary"
          :disabled="!canBatch"
          @click="emit('batch-auto-classify')"
        >
          <Icon :name="batchClassifying ? 'loader' : 'sparkles'" :size="14" :class="{ spin: batchClassifying }" />
          <span>{{ batchClassifying ? '归类中' : '自动归类' }}</span>
        </button>
        <button
          type="button"
          class="hr-sel-btn hr-sel-btn--icon"
          title="退出多选"
          aria-label="退出多选"
          @click="emit('cancel')"
        >
          <Icon name="close" :size="16" />
        </button>
      </div>
    </div>
  </Transition>

  <!-- 批量移动文件夹选择菜单 -->
  <Menu
    v-model:visible="moveMenuVisible"
    :items="moveMenuItems"
    placement="top-start"
    :trigger-ref="moveMenuTrigger"
    :min-width="160"
    @select="onMoveMenuSelect"
  />
</template>

<style scoped>
.hr-sel-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px;
  background: var(--bg-2);
  border-top: 1px solid var(--border);
  flex-shrink: 0;
}

/* 左侧全选 */
.hr-sel-select-all {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  cursor: pointer;
  color: var(--text);
  font-size: 12px;
  transition: background var(--duration-fast) var(--ease-standard);
}

.hr-sel-select-all:hover:not(:disabled) {
  background: var(--card);
}

.hr-sel-select-all:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.hr-sel-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: 1.5px solid var(--border);
  border-radius: 3px;
  background: transparent;
  color: #fff;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
}

.hr-sel-check.checked {
  background: var(--primary);
  border-color: var(--primary);
}

.hr-sel-check.partial {
  border-color: var(--primary);
  background: var(--primary);
}

.hr-sel-check.partial::after {
  content: '';
  width: 8px;
  height: 2px;
  background: #fff;
  border-radius: 1px;
}

.hr-sel-count {
  font-weight: 500;
  color: var(--primary);
}

/* 右侧操作 */
.hr-sel-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.hr-sel-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 5px 10px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.hr-sel-btn:hover:not(:disabled) {
  background: var(--card);
}

.hr-sel-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.hr-sel-btn--primary {
  color: var(--primary);
}

.hr-sel-btn--primary:hover:not(:disabled) {
  background: rgba(74, 126, 255, 0.1);
}

.hr-sel-btn--icon {
  padding: 5px;
}

/* loader 旋转 */
.spin :deep(svg) {
  animation: hr-sel-spin 0.8s linear infinite;
}

@keyframes hr-sel-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
