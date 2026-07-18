<script setup lang="ts">
/**
 * ContextRing 上下文使用量圆环
 * SVG 圆环可视化当前对话上下文占用比例。
 *
 * - used / max 比例映射到圆环填充弧度
 * - 颜色分级：< 70% 绿 / 70-90% 橙 / > 90% 红
 * - 尺寸默认 18×18px，可通过 size 调整
 */
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    /** 已使用量 */
    used: number
    /** 最大量 */
    max: number
    /** 直径 px，默认 18 */
    size?: number | string
  }>(),
  {
    size: 18,
  },
)

// 比例：clamp 到 [0, 1]，max 为 0 时显示 0
const ratio = computed(() => {
  if (!props.max || props.max <= 0) return 0
  const r = props.used / props.max
  if (r < 0) return 0
  if (r > 1) return 1
  return r
})

// 颜色分级
const color = computed(() => {
  const r = ratio.value
  if (r > 0.9) return '#e53935' // 红
  if (r >= 0.7) return '#fb8c00' // 橙
  return '#43a047' // 绿
})

// SVG 几何参数：viewBox 36×36，圆心 (18,18)，半径 14，描边 4
const R = 14
const CIRCUMFERENCE = 2 * Math.PI * R
const trackStyle = computed(() => ({
  stroke: 'var(--border, rgba(0,0,0,0.1))',
  strokeWidth: 4,
  fill: 'none',
}))
const progressStyle = computed(() => ({
  stroke: color.value,
  strokeWidth: 4,
  fill: 'none',
  strokeLinecap: 'round' as const,
  strokeDasharray: `${(ratio.value * CIRCUMFERENCE).toFixed(2)} ${CIRCUMFERENCE.toFixed(2)}`,
  // 从顶部 12 点开始顺时针：rotate(-90) 围绕圆心 (18,18)
  transform: 'rotate(-90 18 18)',
  transition: 'stroke-dasharray 0.3s ease, stroke 0.3s ease',
}))

const sizeStyle = computed(() => {
  const s = typeof props.size === 'number' ? `${props.size}px` : props.size
  return { width: s, height: s }
})
</script>

<template>
  <span class="context-ring" :style="sizeStyle" aria-hidden="true">
    <svg viewBox="0 0 36 36" class="context-ring-svg">
      <circle cx="18" cy="18" :r="R" v-bind="trackStyle" />
      <circle cx="18" cy="18" :r="R" v-bind="progressStyle" />
    </svg>
  </span>
</template>

<style scoped>
.context-ring {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  line-height: 0;
}

.context-ring-svg {
  width: 100%;
  height: 100%;
  display: block;
}
</style>
