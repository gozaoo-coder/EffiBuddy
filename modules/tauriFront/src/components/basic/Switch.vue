<script setup lang="ts">
/**
 * Switch 开关组件（类似 iOS toggle）
 * 参考 HarmonyOS NEXT 设计规范，统一使用 design tokens
 * 三档尺寸：sm 36×20 / md 44×24 / lg 52×28
 * 开时轨道 primary 色，关时 card-2 色，滑块白色滑动
 */
import { computed } from 'vue'

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
    <span class="switch-knob"></span>
  </button>
</template>
