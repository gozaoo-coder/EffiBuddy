<script setup lang="ts">
/**
 * ContextRing 上下文使用量圆环
 * SVG 圆环可视化当前对话上下文占用比例。
 *
 * - used / max 比例映射到圆环填充弧度
 * - 颜色分级：< 60% 绿 / 60-80% 琥珀 / 80-92% 橙 / > 92% 红（濒临耗尽带光晕）
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

// 颜色分级（更亮更饱和，状态区分更明确）
const color = computed(() => {
  const r = ratio.value
  if (r > 0.92) return '#ff4d4f' // 红：濒临耗尽
  if (r >= 0.8) return '#ff8f1f' // 橙：偏高
  if (r >= 0.6) return '#f5b400' // 琥珀：接近警戒
  return '#2fd96e' // 绿：充裕
})

// SVG 几何参数：viewBox 36×36，圆心 (18,18)，半径 13.5，描边 8（更粗的环）
// circle 设 pathLength="100"，dasharray 直接按百分比映射（如 30% → 30），
// 避免自行计算周长带来的偏差；linecap 用 butt 保证弧长与真实占比严格一致
// （round 端帽会在两端各外扩 strokeWidth/2，小占比时视觉上明显偏大）
const R = 13.5
const STROKE = 8
const trackStyle = computed(() => ({
  stroke: 'var(--ring-track, rgba(128, 128, 145, 0.28))',
  strokeWidth: STROKE,
  fill: 'none',
}))
const progressStyle = computed(() => ({
  stroke: color.value,
  strokeWidth: STROKE,
  fill: 'none',
  strokeLinecap: 'butt' as const,
  strokeDasharray: `${(ratio.value * 100).toFixed(2)} 100`,
  // 从顶部 12 点开始顺时针：rotate(-90) 围绕圆心 (18,18)
  transform: 'rotate(-90 18 18)',
  // 濒临耗尽时加光晕提醒
  filter:
    ratio.value > 0.92 ? `drop-shadow(0 0 3px ${color.value})` : 'none',
}))

const sizeStyle = computed(() => {
  const s = typeof props.size === 'number' ? `${props.size}px` : props.size
  return { width: s, height: s }
})
</script>

<template>
  <span class="context-ring" :style="sizeStyle" aria-hidden="true">
    <svg viewBox="0 0 36 36" class="context-ring-svg">
      <circle cx="18" cy="18" :r="R" pathLength="100" v-bind="trackStyle" />
      <circle cx="18" cy="18" :r="R" pathLength="100" v-bind="progressStyle" />
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

/* transition 必须走 CSS 而非 SVG 属性才能生效 */
.context-ring-svg circle:last-child {
  transition: stroke-dasharray 0.3s ease, stroke 0.3s ease;
}
</style>
