<script setup lang="ts">
/**
 * Switch 开关组件（类似 iOS toggle）
 * 参考 HarmonyOS NEXT 设计规范，统一使用 design tokens
 * 三档尺寸：sm 36×20 / md 44×24 / lg 52×28
 * 开时轨道 primary 色，关时 card-2 色，滑块白色滑动
 *
 * 微交互：滑块位移由 anime.js v4 驱动，替代 CSS transform 过渡。
 * 轨道背景色仍由 CSS transition 负责。
 */
import { computed, ref, watch } from 'vue'
import { animate } from 'animejs'

export type SwitchSize = 'sm' | 'md' | 'lg'

const props = withDefaults(
  defineProps<{
    /** 当前开关状态（v-model） */
    modelValue?: boolean
    /** 禁用 */
    disabled?: boolean
    /** 尺寸：sm 36×20 / md 44×24 / lg 52×28 */
    size?: SwitchSize
  }>(),
  {
    modelValue: false,
    disabled: false,
    size: 'md',
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
  (e: 'change', v: boolean): void
}>()

// class 组合
const classes = computed(() => {
  const list: string[] = ['switch']
  list.push(`switch--${props.size}`)
  if (props.modelValue) list.push('switch--on')
  if (props.disabled) list.push('switch--disabled')
  return list
})

// 滑块位移：根据尺寸决定（轨道宽 - 滑块直径 - 2*边距）
// sm: 36-16-4=16 ; md: 44-20-4=20 ; lg: 52-24-4=24
const sizeTravel: Record<SwitchSize, number> = { sm: 16, md: 20, lg: 24 }

const knobEl = ref<HTMLSpanElement | null>(null)

// 监听 modelValue 变化，用 anime.js 驱动滑块位移
// false→true: translateX 0→travel；true→false: travel→0
watch(
  () => props.modelValue,
  (next, prev) => {
    if (next === prev || !knobEl.value) return
    const travel = sizeTravel[props.size]
    const from = next ? 0 : travel
    const to = next ? travel : 0
    animate(knobEl.value, {
      translateX: [from, to],
      duration: 280,
      ease: 'out(3)',
      onComplete: () => {
        // 清理内联 transform：CSS 已根据 switch--on class 处于正确状态
        if (knobEl.value) knobEl.value.style.transform = ''
      },
    })
  },
)

function toggle() {
  if (props.disabled) return
  const next = !props.modelValue
  emit('update:modelValue', next)
  emit('change', next)
}
</script>

<template>
  <button
    :class="classes"
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :disabled="disabled"
    @click="toggle"
  >
    <span ref="knobEl" class="switch-knob"></span>
  </button>
</template>
