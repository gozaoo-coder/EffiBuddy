<script setup lang="ts">
/**
 * ToggleButton 状态开关按钮
 * 点击切换 active 状态，支持 v-model
 * active 时显示 primary 色实心，inactive 时显示 normal 样式
 *
 * 微交互：press/release 的 scale 动画由 anime.js v4 驱动，替代 CSS :active。
 * hover 颜色过渡仍由 CSS 负责。
 */
import { computed, ref } from 'vue'
import { animate } from 'animejs'

export type ToggleButtonSize = 'sm' | 'md' | 'lg'
export type ToggleButtonVariant = 'primary' | 'normal'

const props = withDefaults(
  defineProps<{
    /** 当前激活状态（v-model） */
    modelValue?: boolean
    /** 激活时显示文本（可选） */
    activeText?: string
    /** 未激活时显示文本（可选） */
    inactiveText?: string
    /** 尺寸：sm 28 / md 36 / lg 44 */
    size?: ToggleButtonSize
    /** 激活时变体：primary 实心 primary / normal 实心 card */
    variant?: ToggleButtonVariant
    /** 禁用 */
    disabled?: boolean
  }>(),
  {
    modelValue: false,
    size: 'md',
    variant: 'primary',
    disabled: false,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
  (e: 'change', v: boolean): void
}>()

// 显示文本：根据当前状态选择
const displayText = computed(() => {
  if (props.modelValue) return props.activeText ?? ''
  return props.inactiveText ?? ''
})

// class 组合
const classes = computed(() => {
  const list: string[] = ['toggle-btn']
  list.push(`toggle-btn--${props.size}`)
  if (props.modelValue) {
    list.push('toggle-btn--active')
    list.push(`toggle-btn--active-${props.variant}`)
  }
  if (props.disabled) list.push('toggle-btn--disabled')
  return list
})

function onClick() {
  if (props.disabled) return
  const next = !props.modelValue
  emit('update:modelValue', next)
  emit('change', next)
}

// ---- anime.js v4 微交互：press/release scale ----
const btnEl = ref<HTMLButtonElement | null>(null)
const pressed = ref(false)

function onPointerDown() {
  if (props.disabled || !btnEl.value) return
  pressed.value = true
  animate(btnEl.value, {
    scale: [1, 0.97],
    duration: 120,
    ease: 'out(3)',
  })
}

function onPointerUp() {
  if (!pressed.value || !btnEl.value) return
  pressed.value = false
  animate(btnEl.value, {
    scale: [0.97, 1],
    duration: 150,
    ease: 'out(3)',
    onComplete: () => {
      if (btnEl.value) btnEl.value.style.transform = ''
    },
  })
}

function onPointerLeave() {
  if (!pressed.value || !btnEl.value) return
  pressed.value = false
  animate(btnEl.value, {
    scale: [0.97, 1],
    duration: 150,
    ease: 'out(3)',
    onComplete: () => {
      if (btnEl.value) btnEl.value.style.transform = ''
    },
  })
}
</script>

<template>
  <button
    ref="btnEl"
    :class="classes"
    :disabled="disabled"
    :aria-pressed="modelValue"
    @click="onClick"
    @pointerdown="onPointerDown"
    @pointerup="onPointerUp"
    @pointerleave="onPointerLeave"
  >
    <span v-if="displayText" class="toggle-btn-label">{{ displayText }}</span>
    <slot v-else />
  </button>
</template>
