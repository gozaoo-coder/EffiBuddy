<script setup lang="ts">
/**
 * WidgetDesktop —— 桌面小组件容器
 *
 * 管理所有桌面小组件的顶层容器，提供：
 * - 网格布局 + 自由拖拽模式切换
 * - 堆叠容器嵌入
 * - 小组件注册/反注册
 * - 持久化布局状态
 *
 * 使用方式：
 * ```vue
 * <WidgetDesktop
 *   :widgets="myWidgets"
 *   layout="grid"
 *   @widget-click="onWidgetClick"
 * >
 *   <template #card="{ item }">
 *     <MyWidget :data="item" />
 *   </template>
 * </WidgetDesktop>
 * ```
 */
import { ref, computed, provide } from 'vue'
import { WidgetSize, WIDGET_SIZE_MAP } from '../../types/widget'
import type { WidgetItem, StackEvent } from '../../types/widget'
import WidgetGrid from './WidgetGrid.vue'
import WidgetStack from './WidgetStack.vue'
import WidgetCard from './WidgetCard.vue'

export type DesktopLayout = 'grid' | 'stack' | 'free'

const props = withDefaults(
  defineProps<{
    /** 小组件列表 */
    widgets?: WidgetItem[]
    /** 布局模式 */
    layout?: DesktopLayout
    /** 网格列数 */
    columns?: number
    /** 网格间距 */
    gap?: string
    /** 堆叠预设 */
    stackPreset?: string
    /** 堆叠卡片宽度 */
    stackWidth?: string
    /** 堆叠卡片高度 */
    stackHeight?: string
    /** 堆叠容器跟随顶层卡片尺寸自动变化 */
    stackAutoSize?: boolean
    /** 堆叠最大可见层数 */
    stackMaxVisible?: number
    /** 堆叠拖拽灵敏度阈值（px） */
    stackSensitivity?: number
    /** 堆叠卡片随机旋转（±deg） */
    stackRandomRotation?: boolean
    /** 堆叠点击送回底部 */
    stackSendToBackOnClick?: boolean
    /** 堆叠交互模式 */
    stackInteractionMode?: 'send-to-back' | 'bring-to-front'
    /** 堆叠自动轮播 */
    stackAutoplay?: boolean
    /** 堆叠自动轮播间隔（ms） */
    stackAutoplayDelay?: number
    /** 堆叠悬停暂停轮播 */
    stackPauseOnHover?: boolean
    /** 堆叠弹簧刚度 */
    stackStiffness?: number
    /** 堆叠弹簧阻尼 */
    stackDamping?: number
    /** 堆叠 3D 倾斜强度（deg） */
    stackTiltAmount?: number
    /** 堆叠拖拽弹性系数 */
    stackDragElastic?: number
    /** 堆叠仅顶层可拖 */
    stackTopOnlyDraggable?: boolean
    /** 堆叠循环模式 */
    stackLoop?: boolean
    /** 是否显示添加按钮 */
    showAddButton?: boolean
  }>(),
  {
    widgets: () => [],
    layout: 'grid',
    columns: 4,
    gap: '12px',
    stackPreset: 'fan',
    stackWidth: '320px',
    stackHeight: '420px',
    stackAutoSize: false,
    showAddButton: false,
  },
)

const emit = defineEmits<{
  (e: 'widget-click', id: string): void
  (e: 'widget-close', id: string): void
  (e: 'widget-drag-start', id: string): void
  (e: 'widget-drag-end', id: string, x: number, y: number): void
  (e: 'stack-event', event: StackEvent): void
  (e: 'add-widget'): void
  (e: 'update:layout', layout: DesktopLayout): void
}>()

const stackRef = ref<InstanceType<typeof WidgetStack> | null>(null)

// 是否处于堆叠模式
const isStack = computed(() => props.layout === 'stack')
const isGrid = computed(() => props.layout === 'grid' || props.layout === 'free')

// 将 WidgetItem 转换为 WidgetStack 的 StackItem
const stackItems = computed(() =>
  props.widgets.map((w) => ({
    id: w.id,
    title: w.title,
    ...w.props,
  })),
)

function onCardClick(id: string) {
  emit('widget-click', id)
}

function onCardClose(id: string) {
  emit('widget-close', id)
}

function onCardDragStart(id: string) {
  emit('widget-drag-start', id)
}

function onCardDragEnd(id: string, _x: number, _y: number, _flicked: boolean) {
  emit('widget-drag-end', id, _x, _y)
}

function onStackEvent(evt: StackEvent) {
  emit('stack-event', evt)
}

// 网格尺寸 span 映射（WIDGET_SIZE_MAP）
function getGridSpanStyle(size: WidgetSize) {
  const span = WIDGET_SIZE_MAP[size] ?? WIDGET_SIZE_MAP[WidgetSize.MEDIUM]
  return {
    gridColumn: `span ${Math.min(span.cols, props.columns)}`,
    gridRow: `span ${Math.min(span.rows, props.columns)}`,
  }
}

function handleAddWidget() {
  emit('add-widget')
}

// 切换到堆叠布局
function switchToStack() {
  emit('update:layout', 'stack')
}

// 切换到网格布局
function switchToGrid() {
  emit('update:layout', 'grid')
}

defineExpose({ stackRef })
</script>

<template>
  <div class="widget-desktop">
    <!-- 布局切换工具栏 -->
    <div class="widget-desktop__toolbar">
      <div class="widget-desktop__layout-tabs">
        <button
          :class="['widget-desktop__tab', { 'widget-desktop__tab--active': isGrid }]"
          @click="switchToGrid"
        >
          网格
        </button>
        <button
          :class="['widget-desktop__tab', { 'widget-desktop__tab--active': isStack }]"
          @click="switchToStack"
        >
          堆叠
        </button>
      </div>
      <button
        v-if="showAddButton"
        class="widget-desktop__add-btn"
        @click="handleAddWidget"
      >
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M7 2V12M2 7H12" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        </svg>
        <span>添加</span>
      </button>
    </div>

    <!-- 网格布局 -->
    <div v-if="isGrid" class="widget-desktop__grid-area">
      <WidgetGrid
        :columns="columns"
        :gap="gap"
        class="widget-desktop__grid"
      >
        <WidgetCard
          v-for="w in widgets"
          :key="w.id"
          :card-id="w.id"
          :title="w.title"
          :size="w.size"
          :closable="w.closable ?? true"
          :style="getGridSpanStyle(w.size)"
          @click="onCardClick(w.id)"
          @close="onCardClose"
          @drag-start="onCardDragStart"
          @drag-end="onCardDragEnd"
        >
          <slot name="card" :item="w" :size="w.size">
            <div class="widget-desktop__default-card">
              <p class="widget-desktop__card-title">{{ w.title }}</p>
              <p class="widget-desktop__card-id">{{ w.id }}</p>
            </div>
          </slot>
        </WidgetCard>
      </WidgetGrid>
    </div>

    <!-- 堆叠布局 -->
    <div v-else class="widget-desktop__stack-area">
      <WidgetStack
        ref="stackRef"
        :preset="stackPreset"
        :width="stackWidth"
        :height="stackHeight"
        :auto-size="stackAutoSize"
        :max-visible="stackMaxVisible"
        :sensitivity="stackSensitivity"
        :random-rotation="stackRandomRotation"
        :send-to-back-on-click="stackSendToBackOnClick"
        :interaction-mode="stackInteractionMode"
        :autoplay="stackAutoplay"
        :autoplay-delay="stackAutoplayDelay"
        :pause-on-hover="stackPauseOnHover"
        :stiffness="stackStiffness"
        :damping="stackDamping"
        :tilt-amount="stackTiltAmount"
        :drag-elastic="stackDragElastic"
        :top-only-draggable="stackTopOnlyDraggable"
        :loop="stackLoop"
        @card-click="onCardClick"
        @card-dismissed="onCardClose"
        @stack-event="onStackEvent"
      >
        <template #card="{ item }">
          <slot name="stack-card" :item="item">
            <div class="widget-desktop__default-card">
              <p class="widget-desktop__card-title">{{ item.title }}</p>
              <p class="widget-desktop__card-id">{{ item.id }}</p>
            </div>
          </slot>
        </template>
      </WidgetStack>
    </div>
  </div>
</template>

<style scoped>
.widget-desktop {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  overflow: hidden;
}

/* 工具栏 */
.widget-desktop__toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border, rgba(255, 255, 255, 0.09));
  flex-shrink: 0;
}

.widget-desktop__layout-tabs {
  display: flex;
  gap: 2px;
  background: var(--bg-2, rgb(32, 32, 36));
  border-radius: var(--radius-sm, 3px);
  padding: 2px;
}

.widget-desktop__tab {
  padding: 4px 12px;
  font-size: 11px;
  font-weight: 500;
  border: none;
  border-radius: 2px;
  background: transparent;
  color: var(--muted, #9ba2b2);
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}

.widget-desktop__tab:hover {
  color: var(--text, #e9ebf1);
}

.widget-desktop__tab--active {
  background: var(--card, rgb(40, 40, 45));
  color: var(--text, #e9ebf1);
}

.widget-desktop__add-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  font-size: 11px;
  font-weight: 500;
  border: 1px solid var(--border, rgba(255, 255, 255, 0.09));
  border-radius: var(--radius-sm, 3px);
  background: var(--card, rgb(40, 40, 45));
  color: var(--text, #e9ebf1);
  cursor: pointer;
  transition: background 0.15s;
}

.widget-desktop__add-btn:hover {
  background: var(--hover, rgba(255, 255, 255, 0.1));
}

/* 网格区域 */
.widget-desktop__grid-area {
  flex: 1;
  overflow: auto;
  padding: 12px;
}

.widget-desktop__grid {
  min-height: 100%;
}

/* 堆叠区域 */
.widget-desktop__stack-area {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}

/* 默认卡片内容 */
.widget-desktop__default-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px;
}

.widget-desktop__card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text, #e9ebf1);
  margin: 0;
}

.widget-desktop__card-id {
  font-size: 10px;
  color: var(--muted, #9ba2b2);
  margin: 0;
  font-family: monospace;
}
</style>