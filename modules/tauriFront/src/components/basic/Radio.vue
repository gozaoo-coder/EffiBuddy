<script setup lang="ts">
/**
 * Radio 单选框组件（单个，通常配合 RadioGroup 使用）
 * 参考 HarmonyOS NEXT 设计规范
 * 圆形指示器 18px，选中时内圈实心 primary 色（伪元素缩放出现）
 * label 文字在右侧，点击整行切换；disabled 时 opacity 0.5
 * 通过 inject 自动接入 RadioGroup 上下文（也支持独立使用）
 *
 * 微交互：选中时内圈 dot 用 anime.js v4 做 scale(0)→scale(1) 动画。
 * 原 CSS 用 ::after 伪元素（保留作为 fallback），这里改为对真实
 * <span class="radio-dot"> 做动画（anime.js 无法直接操作伪元素）。
 */
import { computed, inject, ref, watch } from 'vue'
import { animate } from 'animejs'
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

// ---- anime.js v4 微交互：选中圈 dot scale 动画 ----
// 使用真实 span.radio-dot 替代 ::after 伪元素，便于 anime.js 直接操作。
// 通过 watch 监听 isChecked 变化，在 Vue 更新 DOM class 之前（flush:'pre'）
// 设置内联 transform，避免 CSS class 切换造成的闪烁。
const dotEl = ref<HTMLSpanElement | null>(null)

watch(
  isChecked,
  (checked) => {
    if (!dotEl.value) return
    if (checked) {
      // 选中：scale 0 → 1
      animate(dotEl.value, {
        transform: ['scale(0)', 'scale(1)'],
        duration: 280,
        ease: 'out(3)',
        onComplete: () => {
          // 清理内联 transform：CSS 已通过 .radio--checked 处于 scale(1)
          if (dotEl.value) dotEl.value.style.transform = ''
        },
      })
    } else {
      // 取消选中：scale 1 → 0
      animate(dotEl.value, {
        transform: ['scale(1)', 'scale(0)'],
        duration: 220,
        ease: 'out(3)',
        onComplete: () => {
          if (dotEl.value) dotEl.value.style.transform = ''
        },
      })
    }
  },
)
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
    <span class="radio-indicator" aria-hidden="true">
      <span ref="dotEl" class="radio-dot"></span>
    </span>
    <span v-if="label || $slots.default" class="radio-label">
      <slot>{{ label }}</slot>
    </span>
  </label>
</template>

<style scoped>
/*
 * radio-dot：真实的选中圆点（替代 ::after 伪元素，便于 anime.js 操作）。
 * 使用 inset:0 + margin:auto 居中，避免 translate 与 anime.js transform 冲突。
 * 初始 scale(0)，选中时由 .radio--checked 切到 scale(1)（CSS 兜底），
 * anime.js watch 会在 class 切换前设置内联 transform 驱动动画。
 */
.radio-dot {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  left: 0;
  margin: auto;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--primary);
  transform: scale(0);
  pointer-events: none;
}

.radio--checked .radio-dot {
  transform: scale(1);
}
</style>
