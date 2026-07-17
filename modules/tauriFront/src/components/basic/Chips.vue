<script setup lang="ts">
/**
 * Chips 操作块组件（块状入口，常用于标签/收件人）
 * 参考 HarmonyOS NEXT 设计规范
 * 胶囊形（radius-full），sm 高度 28px / md 高度 36px
 * 左侧 icon/image，中间 label，右侧 × 删除按钮（removable 时）
 * selected 时 primary 色边框 + 淡 primary 背景
 * 点击使用 anime.js v4 做 scale 弹性反馈（1 → 0.94 → 1）
 */
import { computed, ref } from 'vue'
import { animate } from 'animejs'

export type ChipsSize = 'sm' | 'md'

const props = withDefaults(
  defineProps<{
    /** 标签文本 */
    label: string
    /** 图标（emoji 或字符） */
    icon?: string
    /** 图片 URL */
    image?: string
    /** 是否可删除（显示 × 按钮） */
    removable?: boolean
    /** 选中状态（v-model:selected） */
    selected?: boolean
    /** 尺寸：sm 28 / md 36 */
    size?: ChipsSize
    /** 禁用 */
    disabled?: boolean
  }>(),
  {
    removable: false,
    selected: false,
    size: 'md',
    disabled: false,
  },
)

const emit = defineEmits<{
  (e: 'click', ev: MouseEvent): void
  (e: 'remove', ev: MouseEvent): void
  (e: 'update:selected', v: boolean): void
}>()

const classes = computed(() => {
  const list: string[] = ['chips']
  list.push(`chips--${props.size}`)
  if (props.selected) list.push('chips--selected')
  if (props.disabled) list.push('chips--disabled')
  if (props.removable) list.push('chips--removable')
  return list
})

// 根元素引用，用于触发 anime.js 弹性反馈
const rootEl = ref<HTMLElement | null>(null)

function onClick(ev: MouseEvent) {
  if (props.disabled) return
  emit('click', ev)
  // 点击切换 selected（如果用 v-model:selected）
  emit('update:selected', !props.selected)
  // 弹性反馈：1 → 0.94 → 1 三帧回弹
  if (rootEl.value) {
    animate(rootEl.value, {
      scale: [1, 0.94, 1],
      duration: 220,
      ease: 'out(3)',
      onComplete: () => {
        // 清理内联 transform，避免影响 hover/selected 等布局状态
        if (rootEl.value) rootEl.value.style.transform = ''
      },
    })
  }
}

function onRemove(ev: MouseEvent) {
  if (props.disabled) return
  ev.stopPropagation()
  emit('remove', ev)
}
</script>

<template>
  <span
    ref="rootEl"
    :class="classes"
    role="button"
    :aria-pressed="selected"
    :aria-disabled="disabled"
    tabindex="0"
    @click="onClick"
  >
    <!-- 图片优先于 icon -->
    <span v-if="image" class="chips-image">
      <img :src="image" :alt="label" />
    </span>
    <span v-else-if="icon" class="chips-icon">{{ icon }}</span>
    <span class="chips-label">{{ label }}</span>
    <button
      v-if="removable"
      class="chips-remove"
      type="button"
      :aria-label="`移除 ${label}`"
      :disabled="disabled"
      @click="onRemove"
    >×</button>
  </span>
</template>
