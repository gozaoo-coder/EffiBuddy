<script setup lang="ts">
/**
 * IconButton 独立图标按钮
 * 专门用于只放图标的场景，正方形（宽=高=size 对应高度）
 * 支持 emoji 或字符作为图标内容
 */
import { computed } from 'vue'

export type IconButtonSize = 'sm' | 'md' | 'lg'
export type IconButtonVariant = 'normal' | 'primary' | 'danger'

const props = withDefaults(
  defineProps<{
    /** 图标内容：emoji 或字符 */
    icon: string
    /** 尺寸：sm 28 / md 36 / lg 44 */
    size?: IconButtonSize
    /** 带容器（card 背景 + border） */
    container?: boolean
    /** 变体：normal 普通 / primary 强调 / danger 危险 */
    variant?: IconButtonVariant
    /** 禁用 */
    disabled?: boolean
  }>(),
  {
    size: 'md',
    container: false,
    variant: 'normal',
    disabled: false,
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

function onClick(ev: MouseEvent) {
  if (props.disabled) return
  emit('click', ev)
}
</script>

<template>
  <button
    :class="classes"
    :disabled="disabled"
    :aria-label="icon"
    @click="onClick"
  >
    <span class="icon-btn-glyph">{{ icon }}</span>
  </button>
</template>
