<script setup lang="ts">
/**
 * Button 通用按钮组件
 * 参考 HarmonyOS NEXT 设计规范，统一使用 design tokens
 * 支持 variant/size/icon-only/container/loading/disabled/block/shape 等配置
 */
import { computed } from 'vue'

export type ButtonVariant = 'primary' | 'normal' | 'text' | 'danger'
export type ButtonSize = 'sm' | 'md' | 'lg'
export type ButtonShape = 'default' | 'capsule' | 'circle'

const props = withDefaults(
  defineProps<{
    /** 变体：primary 强调 / normal 普通 / text 文字 / danger 危险 */
    variant?: ButtonVariant
    /** 尺寸：sm 28 / md 36 / lg 44 */
    size?: ButtonSize
    /** 是否为图标按钮（正方形，宽=高） */
    iconOnly?: boolean
    /** 带容器的图标按钮（card 背景 + border），仅 icon-only 时生效 */
    container?: boolean
    /** 等待状态，显示 spinner 并禁用按钮 */
    loading?: boolean
    /** 禁用 */
    disabled?: boolean
    /** 块级按钮，宽度 100% */
    block?: boolean
    /** 形状：default 圆角 / capsule 胶囊 / circle 圆形（仅 icon-only） */
    shape?: ButtonShape
  }>(),
  {
    variant: 'normal',
    size: 'md',
    iconOnly: false,
    container: false,
    loading: false,
    disabled: false,
    block: false,
    shape: 'default',
  },
)

const emit = defineEmits<{
  (e: 'click', ev: MouseEvent): void
}>()

// 实际禁用状态：disabled 或 loading 都禁用
const isDisabled = computed(() => props.disabled || props.loading)

// class 计算函数：根据 props 组合样式
const classes = computed(() => {
  const list: string[] = ['btn-base']
  list.push(`btn-base--${props.variant}`)
  list.push(`btn-base--${props.size}`)
  if (props.iconOnly) {
    list.push('btn-base--icon')
    if (props.container) list.push('btn-base--container')
  }
  if (props.block) list.push('btn-base--block')
  if (props.shape !== 'default') list.push(`btn-base--${props.shape}`)
  if (props.loading) list.push('btn-base--loading')
  return list
})

function onClick(ev: MouseEvent) {
  if (isDisabled.value) return
  emit('click', ev)
}
</script>

<template>
  <button
    :class="classes"
    :disabled="isDisabled"
    :aria-busy="loading"
    @click="onClick"
  >
    <!-- loading 状态显示 spinner 替换内容 -->
    <span v-if="loading" class="btn-base-spinner" aria-hidden="true"></span>
    <template v-else>
      <!-- icon 插槽 -->
      <span v-if="$slots.icon" class="btn-base-icon">
        <slot name="icon" />
      </span>
      <!-- 默认插槽：按钮文本，icon-only 时隐藏 -->
      <span v-if="!iconOnly && $slots.default" class="btn-base-label">
        <slot />
      </span>
    </template>
  </button>
</template>
