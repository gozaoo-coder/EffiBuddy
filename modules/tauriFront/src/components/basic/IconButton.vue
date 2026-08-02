<script setup lang="ts">
/**
 * IconButton 独立图标按钮
 * 专门用于只放图标的场景，正方形（宽=高=size 对应高度）
 * 支持 emoji 或字符作为图标内容
 *
 * 微交互：press/release 的 scale 动画由 anime.js v4 驱动（按下到 0.92），
 * 替代 CSS :active。hover 颜色过渡仍由 CSS 负责。
 */
import { computed, ref } from 'vue'
import { animate } from 'animejs'
import Icon from '../Icon.vue'

export type IconButtonSize = 'sm' | 'md' | 'lg'
export type IconButtonVariant = 'normal' | 'primary' | 'danger'

const props = withDefaults(
  defineProps<{
    /** 图标内容：emoji 或字符（使用 slot 时可省略） */
    icon?: string
    /** 尺寸：sm 28 / md 36 / lg 44 */
    size?: IconButtonSize
    /** 带容器（card 背景 + border） */
    container?: boolean
    /** 变体：normal 普通 / primary 强调 / danger 危险 */
    variant?: IconButtonVariant
    /** 禁用 */
    disabled?: boolean
    /** 是否显示未读红点（左上角） */
    dot?: boolean
  }>(),
  {
    icon: '',
    size: 'md',
    container: false,
    variant: 'normal',
    disabled: false,
    dot: false,
  },
)

const emit = defineEmits<{
  (e: 'click', ev: MouseEvent): void
}>()

const classes = computed(() => {
    const list: string[] = ['icon-btn']
    list.push(`icon-btn--${props.size}`)
    list.push(`icon-btn--${props.variant}`)
    if (props.container) list.push('icon-btn--container')
    if (props.disabled) list.push('icon-btn--disabled')
    return list
  })

  // icon 为语义名时按按钮尺寸渲染对应图标（与 <Icon> 组件同源）
  const iconSize = computed(() => (props.size === 'sm' ? 16 : props.size === 'lg' ? 22 : 18))

function onClick(ev: MouseEvent) {
  if (props.disabled) return
  emit('click', ev)
}

// ---- anime.js v4 微交互：press/release scale（按下到 0.92）----
const btnEl = ref<HTMLButtonElement | null>(null)
const pressed = ref(false)

function onPointerDown() {
  if (props.disabled || !btnEl.value) return
  pressed.value = true
  animate(btnEl.value, {
    scale: [1, 0.92],
    duration: 120,
    ease: 'out(3)',
  })
}

function onPointerUp() {
  if (!pressed.value || !btnEl.value) return
  pressed.value = false
  animate(btnEl.value, {
    scale: [0.92, 1],
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
    scale: [0.92, 1],
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
    :aria-label="icon ?? ''"
    @click="onClick"
    @pointerdown="onPointerDown"
    @pointerup="onPointerUp"
    @pointerleave="onPointerLeave"
  >
      <span class="icon-btn-glyph">
        <slot>
          <Icon v-if="icon" :name="icon" :size="iconSize" />
        </slot>
      </span>
    <span v-if="dot" class="icon-btn-dot" aria-hidden="true"></span>
  </button>
</template>
