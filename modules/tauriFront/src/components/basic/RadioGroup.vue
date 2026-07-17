<script setup lang="ts">
/**
 * RadioGroup 单选组组件
 * 通过 provide/inject 统一管理子 Radio 的 modelValue
 * Radio 子组件通过 inject(radioGroupKey) 自动接入组上下文
 * slot 放多个 Radio 子组件
 */
import { provide, toRef } from 'vue'
import type { InjectionKey, Ref } from 'vue'

/** RadioGroup 注入给子 Radio 的上下文类型 */
export interface RadioGroupContext {
  /** 当前选中值（响应式 ref） */
  modelValue: Ref<unknown>
  /** name 属性（响应式 ref，可能为 undefined） */
  name: Ref<string | undefined>
  /** 是否禁用整组 */
  disabled: Ref<boolean | undefined>
  /** 子项选中触发：更新 modelValue 并 emit change */
  select: (value: unknown) => void
}

/** provide/inject 的 key（Symbol，避免冲突） */
export const radioGroupKey: InjectionKey<RadioGroupContext> = Symbol('radioGroup')

const props = withDefaults(
  defineProps<{
    /** 当前选中值（v-model） */
    modelValue?: unknown
    /** 表单 name 属性，会下发到所有子 Radio */
    name?: string
    /** 整组禁用 */
    disabled?: boolean
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

// 提供给子 Radio 的上下文
const context: RadioGroupContext = {
  modelValue: toRef(props, 'modelValue'),
  name: toRef(props, 'name'),
  disabled: toRef(props, 'disabled'),
  select(value: unknown) {
    if (props.disabled) return
    if (value === props.modelValue) return
    emit('update:modelValue', value)
    emit('change', value)
  },
}

provide(radioGroupKey, context)
</script>

<template>
  <div class="radio-group" role="radiogroup">
    <slot />
  </div>
</template>
