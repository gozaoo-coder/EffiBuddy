<script setup lang="ts">
/**
 * SegmentedButton 分段按钮（类似 iOS segmented control）
 * 参考 HarmonyOS NEXT 设计规范
 * 多段横排，整体圆角 radius-sm，每段等宽，段间 1px border 分隔
 * 选中段 primary 色背景白字，未选中透明背景 text 色
 * 切换有过渡动效；block 时宽度 100%
 */
import { computed } from 'vue'

export type SegmentedSize = 'sm' | 'md' | 'lg'

export interface SegmentedOption {
  /** 显示文本 */
  label: string
  /** 选项值 */
  value: string | number
  /** 图标（可选，emoji 或字符） */
  icon?: string
}

const props = withDefaults(
  defineProps<{
    /** 当前选中值（v-model） */
    modelValue?: string | number
    /** 选项列表 */
    options: SegmentedOption[]
    /** 尺寸：sm 28 / md 36 / lg 44 */
    size?: SegmentedSize
    /** 禁用 */
    disabled?: boolean
    /** 块级宽度 100% */
    block?: boolean
  }>(),
  {
    modelValue: undefined,
    size: 'md',
    disabled: false,
    block: false,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: string | number): void
  (e: 'change', v: string | number, option: SegmentedOption): void
}>()

const wrapperClasses = computed(() => {
  const list: string[] = ['segmented']
  list.push(`segmented--${props.size}`)
  if (props.block) list.push('segmented--block')
  if (props.disabled) list.push('segmented--disabled')
  return list
})

function onSelect(opt: SegmentedOption) {
  if (props.disabled) return
  if (opt.value === props.modelValue) return
  emit('update:modelValue', opt.value)
  emit('change', opt.value, opt)
}
</script>

<template>
  <div :class="wrapperClasses" role="tablist">
    <button
      v-for="(opt, idx) in options"
      :key="opt.value"
      type="button"
      role="tab"
      :class="[
        'segmented-item',
        {
          'is-selected': opt.value === modelValue,
          'is-first': idx === 0,
          'is-last': idx === options.length - 1,
        },
      ]"
      :disabled="disabled"
      @click="onSelect(opt)"
    >
      <span v-if="opt.icon" class="segmented-item-icon">{{ opt.icon }}</span>
      <span class="segmented-item-label">{{ opt.label }}</span>
    </button>
  </div>
</template>
