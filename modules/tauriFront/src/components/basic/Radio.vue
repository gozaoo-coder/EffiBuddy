<script setup lang="ts">
/**
 * Radio 单选框组件（单个，通常配合 RadioGroup 使用）
 * 参考 HarmonyOS NEXT 设计规范
 * 圆形指示器 18px，选中时内圈实心 primary 色（伪元素缩放出现）
 * label 文字在右侧，点击整行切换；disabled 时 opacity 0.5
 * 通过 inject 自动接入 RadioGroup 上下文（也支持独立使用）
 */
import { computed, inject } from 'vue'
import { radioGroupKey, type RadioGroupContext } from './RadioGroup.vue'

const props = withDefaults(
  defineProps<{
    /** 当前选中值（v-model，独立使用时） */
    modelValue?: unknown
    /** 当前 radio 所代表的值 */
    value: unknown
    /** 标签文本 */
    label?: string
    /** 禁用 */
    disabled?: boolean
    /** name 属性（独立使用时；组内会从 RadioGroup 继承） */
    name?: string
  }>(),
  {
    modelValue: undefined,
    disabled: false,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: unknown): void
  (e: 'change', v: unknown): void
}>()

// 注入 RadioGroup 上下文（可能为 null，表示独立使用）
const group = inject<RadioGroupContext | null>(radioGroupKey, null)

// 是否选中：组内取 group.modelValue，独立取 props.modelValue
const isChecked = computed(() => {
  const current = group ? group.modelValue.value : props.modelValue
  return current === props.value
})

// 实际 name：组内优先用 group.name
const resolvedName = computed(() => (group?.name?.value ?? props.name) ?? '')

// 实际 disabled：组内或自身任一为 true 都禁用
const isDisabled = computed(() => {
  return !!(props.disabled || group?.disabled?.value)
})

const classes = computed(() => {
  const list: string[] = ['radio']
  if (isChecked.value) list.push('radio--checked')
  if (isDisabled.value) list.push('radio--disabled')
  return list
})

function onClick() {
  if (isDisabled.value) return
  // 已选中再点击不重复触发
  if (isChecked.value) return
  if (group) {
    // 组内：交给 group 统一更新
    group.select(props.value)
  } else {
    // 独立使用：自身 emit
    emit('update:modelValue', props.value)
    emit('change', props.value)
  }
}
</script>

<template>
  <label
    :class="classes"
    :aria-checked="isChecked"
    role="radio"
    @click="onClick"
  >
    <input
      class="radio-native"
      type="radio"
      :name="resolvedName"
      :value="String(value)"
      :checked="isChecked"
      :disabled="isDisabled"
      tabindex="-1"
      @click.stop
    />
    <span class="radio-indicator" aria-hidden="true"></span>
    <span v-if="label || $slots.default" class="radio-label">
      <slot>{{ label }}</slot>
    </span>
  </label>
</template>
