<script setup lang="ts">
/**
 * WidgetGrid —— 网格布局组件
 *
 * 将卡片置于 CSS Grid 网格中，支持：
 * - 配置列数和行高
 * - 卡片尺寸枚举映射到 grid span
 * - 卡片拖拽交换位置（完善中）
 * - 进入/离开动画（anime.js Layout）
 * - 响应式列数
 */
import { ref, computed, useSlots } from 'vue'
import { useLayout } from '../../composables/useLayout'
import { WidgetSize, WIDGET_SIZE_MAP } from '../../types/widget'

const props = withDefaults(
  defineProps<{
    /** 网格列数 */
    columns?: number
    /** 网格行高 */
    rowHeight?: string
    /** 网格间距 */
    gap?: string
    /** 是否响应式（根据容器宽度自动调整列数） */
    responsive?: boolean
    /** 最小列宽（响应式时生效） */
    minColumnWidth?: string
  }>(),
  {
    columns: 4,
    rowHeight: 'auto',
    gap: '12px',
    responsive: false,
    minColumnWidth: '180px',
  },
)

const gridRef = ref<HTMLElement | null>(null)

// 动画布局
const { update } = useLayout(gridRef, {
  children: '.widget-grid__item',
  duration: 300,
  ease: 'out(3)',
  enterFrom: { opacity: 0, transform: 'translateY(12px) scale(0.96)' },
  leaveTo: { opacity: 0, transform: 'scale(0.9)' },
})

// 是否响应式
const gridTemplate = computed(() => {
  if (props.responsive) {
    return `repeat(auto-fill, minmax(${props.minColumnWidth}, 1fr))`
  }
  return `repeat(${props.columns}, 1fr)`
})

// 根据 WidgetSize 获取 grid class
function getSizeClass(size: WidgetSize): string {
  const span = WIDGET_SIZE_MAP[size]
  return `widget-grid__item--${size}`
}

// 获取 grid span 样式
function getGridStyle(size: WidgetSize) {
  const span = WIDGET_SIZE_MAP[size]
  return {
    gridColumn: `span ${span.cols}`,
    gridRow: `span ${span.rows}`,
  }
}

defineExpose({ update })
</script>

<template>
  <div
    ref="gridRef"
    class="widget-grid"
    :style="{
      gridTemplateColumns: gridTemplate,
      gridAutoRows: rowHeight,
      gap,
    }"
  >
    <slot />
  </div>
</template>

<style scoped>
.widget-grid {
  display: grid;
  width: 100%;
  align-content: start;
}

.widget-grid__item {
  min-width: 0;
  min-height: 0;
}

/* 尺寸 span 映射——通过内联样式实现，这里只做兜底 */
.widget-grid__item--tiny {
  /* 1×1 */
}
.widget-grid__item--small {
  /* 1×2 */
}
.widget-grid__item--medium {
  /* 2×2 */
}
.widget-grid__item--large {
  /* 2×3 */
}
.widget-grid__item--xlarge {
  /* 3×3 */
}
.widget-grid__item--wide {
  /* 3×1 */
}
.widget-grid__item--tall {
  /* 1×3 */
}
.widget-grid__item--full {
  /* 4×4 */
}
</style>