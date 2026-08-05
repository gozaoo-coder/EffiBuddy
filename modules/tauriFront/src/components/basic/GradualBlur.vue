<script setup lang="ts">
/**
 * GradualBlur —— vue-bits「gradual-blur」渐进模糊层移植版
 *
 * 原理:在容器边缘叠加 N 层 backdrop-filter 模糊层,每层用线性渐变 mask
 * 只暴露一条「色带」,越靠近边缘的层模糊半径越大,从而形成
 * 「从清晰到渐进模糊」的平滑过渡(内容从层下滚过时逐渐虚化)。
 *
 * 相对原版的裁剪:
 * - 移除 mathjs 依赖(模糊值用原生 Math.pow 计算,公式与原版一致);
 * - 仅保留 target="parent" 定位模式,去除 animated/hover/responsive 等扩展项。
 */
import { computed, type CSSProperties, type StyleValue } from 'vue'

export type GradualBlurPosition = 'top' | 'bottom' | 'left' | 'right'
export type GradualBlurCurve = 'linear' | 'bezier' | 'ease-in' | 'ease-out' | 'ease-in-out'

const props = withDefaults(
  defineProps<{
    /** 模糊覆盖层挂靠的边缘 */
    position?: GradualBlurPosition
    /** 基础模糊强度倍率(影响每一层) */
    strength?: number
    /** 覆盖层高度(vertical 位置) */
    height?: string
    /** 覆盖层宽度(horizontal 位置,默认同 height) */
    width?: string
    /** 堆叠模糊层数量:越大渐变越平滑 */
    divCount?: number
    /** 指数渐进:末端模糊更强 */
    exponential?: boolean
    /** 基础 z-index */
    zIndex?: number
    /** 每层不透明度 */
    opacity?: number
    /** 模糊进度分布曲线 */
    curve?: GradualBlurCurve
  }>(),
  {
    position: 'top',
    strength: 2,
    height: '96px',
    width: '',
    divCount: 5,
    exponential: false,
    zIndex: 20,
    opacity: 1,
    curve: 'bezier',
  },
)

const CURVE_FUNCTIONS: Record<GradualBlurCurve, (p: number) => number> = {
  linear: (p) => p,
  bezier: (p) => p * p * (3 - 2 * p),
  'ease-in': (p) => p * p,
  'ease-out': (p) => 1 - Math.pow(1 - p, 2),
  'ease-in-out': (p) => (p < 0.5 ? 2 * p * p : 1 - Math.pow(-2 * p + 2, 2) / 2),
}

const GRADIENT_DIRECTIONS: Record<GradualBlurPosition, string> = {
  top: 'to top',
  bottom: 'to bottom',
  left: 'to left',
  right: 'to right',
}

/** 逐层计算 mask 色带与模糊半径(与原版 blurDivs 同公式,rem → px ×16) */
const layers = computed<Array<{ style: CSSProperties }>>(() => {
  const divs: Array<{ style: CSSProperties }> = []
  const increment = 100 / props.divCount
  const curve = CURVE_FUNCTIONS[props.curve]
  const direction = GRADIENT_DIRECTIONS[props.position]

  for (let i = 1; i <= props.divCount; i++) {
    let progress = i / props.divCount
    progress = curve(progress)

    const blurRem = props.exponential
      ? Math.pow(2, progress * 4) * 0.0625 * props.strength
      : 0.0625 * (progress * props.divCount + 1) * props.strength
    const blurPx = blurRem * 16

    const p1 = Math.round((increment * i - increment) * 10) / 10
    const p2 = Math.round(increment * i * 10) / 10
    const p3 = Math.round((increment * i + increment) * 10) / 10
    const p4 = Math.round((increment * i + increment * 2) * 10) / 10

    let gradient = `transparent ${p1}%, black ${p2}%`
    if (p3 <= 100) gradient += `, black ${p3}%`
    if (p4 <= 100) gradient += `, transparent ${p4}%`

    divs.push({
      style: {
        maskImage: `linear-gradient(${direction}, ${gradient})`,
        WebkitMaskImage: `linear-gradient(${direction}, ${gradient})`,
        backdropFilter: `blur(${blurPx.toFixed(2)}px)`,
        WebkitBackdropFilter: `blur(${blurPx.toFixed(2)}px)`,
        opacity: props.opacity,
      },
    })
  }
  return divs
})

const containerStyle = computed<StyleValue>(() => {
  const isVertical = props.position === 'top' || props.position === 'bottom'
  const base: CSSProperties & Record<string, string | number> = {
    position: 'absolute',
    pointerEvents: 'none',
    zIndex: props.zIndex,
  }
  if (isVertical) {
    base.height = props.height
    base.width = props.width || '100%'
    base[props.position] = '0'
    base.left = '0'
    base.right = '0'
  } else {
    base.width = props.width || props.height
    base.height = '100%'
    base[props.position] = '0'
    base.top = '0'
    base.bottom = '0'
  }
  return base
})
</script>

<template>
  <div class="gradual-blur" :style="containerStyle" aria-hidden="true">
    <div class="gradual-blur__inner">
      <div
        v-for="(layer, index) in layers"
        :key="index"
        class="gradual-blur__layer"
        :style="layer.style"
      />
    </div>
  </div>
</template>

<style scoped>
.gradual-blur {
  isolation: isolate;
}

.gradual-blur__inner {
  position: relative;
  width: 100%;
  height: 100%;
}

.gradual-blur__layer {
  position: absolute;
  inset: 0;
}
</style>
