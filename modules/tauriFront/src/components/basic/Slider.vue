<script setup lang="ts">
/**
 * Slider 滑动条组件
 * 参考 HarmonyOS NEXT 设计规范，使用原生 input[type=range] + 自定义样式
 * 轨道高度 4/6/8px（sm/md/lg），已填充部分 primary 色
 * 滑块圆形 16/20/24px，hover 1.1、active 1.2 缩放
 * showValue 时右侧显示数值（宽度 40px，muted 色）
 *
 * 微交互：thumb 是伪元素 ::-webkit-slider-thumb，无法直接 ref，
 * 因此保留 CSS 的 thumb hover/active scale。改为对 input 元素做
 * focus 反馈动画（轻微上浮 + 亮度提升），blur 时还原。
 * 轨道渐变（--slider-percent）保留 CSS。
 */
import { computed, ref } from 'vue'
import { animate } from 'animejs'

export type SliderSize = 'sm' | 'md' | 'lg'

const props = withDefaults(
  defineProps<{
    /** 当前值（v-model） */
    modelValue?: number
    /** 最小值 */
    min?: number
    /** 最大值 */
    max?: number
    /** 步长 */
    step?: number
    /** 禁用 */
    disabled?: boolean
    /** 显示当前数值 */
    showValue?: boolean
    /** 尺寸：sm 16 / md 20 / lg 24（滑块直径） */
    size?: SliderSize
  }>(),
  {
    modelValue: 0,
    min: 0,
    max: 100,
    step: 1,
    disabled: false,
    showValue: false,
    size: 'md',
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: number): void
  (e: 'change', v: number): void
}>()

// 已填充部分的百分比，通过 CSS 变量传给伪元素（--slider-percent）
const percent = computed(() => {
  const range = props.max - props.min
  if (range <= 0) return 0
  const v = Math.min(props.max, Math.max(props.min, props.modelValue))
  return ((v - props.min) / range) * 100
})

// 通过 CSS 自定义属性传递百分比给伪元素
const sliderStyle = computed(() => ({
  '--slider-percent': `${percent.value}%`,
}))

const wrapperClasses = computed(() => {
  const list: string[] = ['slider']
  list.push(`slider--${props.size}`)
  if (props.disabled) list.push('slider--disabled')
  return list
})

function onInput(e: Event) {
  const target = e.target as HTMLInputElement
  const v = Number(target.value)
  emit('update:modelValue', v)
}

function onChange(e: Event) {
  const target = e.target as HTMLInputElement
  const v = Number(target.value)
  emit('change', v)
}

// ---- anime.js v4 微交互：focus 时 input 轻微上浮 + 亮度提升 ----
// thumb 是伪元素无法直接 ref，保留 CSS thumb scale；
// 这里对 input 元素做 focus 反馈，作为 track 的"高亮"提示。
const inputEl = ref<HTMLInputElement | null>(null)

function onFocus() {
  if (props.disabled || !inputEl.value) return
  animate(inputEl.value, {
    translateY: [0, -1],
    filter: ['brightness(1)', 'brightness(1.1)'],
    duration: 200,
    ease: 'out(3)',
  })
}

function onBlur() {
  if (!inputEl.value) return
  animate(inputEl.value, {
    translateY: [-1, 0],
    filter: ['brightness(1.1)', 'brightness(1)'],
    duration: 200,
    ease: 'out(3)',
    onComplete: () => {
      if (inputEl.value) {
        inputEl.value.style.transform = ''
        inputEl.value.style.filter = ''
      }
    },
  })
}
</script>

<template>
  <div :class="wrapperClasses">
    <input
      ref="inputEl"
      class="slider-input"
      type="range"
      :min="min"
      :max="max"
      :step="step"
      :value="modelValue"
      :disabled="disabled"
      :style="sliderStyle"
      @input="onInput"
      @change="onChange"
      @focus="onFocus"
      @blur="onBlur"
    />
    <span v-if="showValue" class="slider-value">{{ modelValue }}</span>
  </div>
</template>
