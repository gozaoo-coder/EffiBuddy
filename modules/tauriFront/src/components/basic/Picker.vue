<script setup lang="ts">
/**
 * Picker 选择器组件（滚轮式，类似移动端时间选择器）
 * 参考 HarmonyOS NEXT 设计规范
 * 点击 trigger 展开半模态面板（从底部滑入，Teleport to body，z-index --z-sheet）
 * 滚轮用 scroll-snap 实现：scroll-snap-type: y mandatory，每项 scroll-snap-align: center
 * 选中项 primary 色加粗；顶部标题栏 + 取消/确定按钮
 * 遮罩 rgba(0,0,0,0.45)，点击遮罩取消
 * 遮罩 fade 与面板从底部滑入使用 anime.js v4（通过 useAnimeTransition）
 */
import { ref, computed, watch, nextTick, onUnmounted } from 'vue'
import { useAnimeTransition } from '../../composables/useAnimeTransition'

export interface PickerOption {
  /** 显示文本 */
  label: string
  /** 选项值 */
  value: string | number
}

const props = withDefaults(
  defineProps<{
    /** 当前选中值（v-model） */
    modelValue?: string | number
    /** 选项列表 */
    options: PickerOption[]
    /** 面板标题 */
    title?: string
    /** 确定按钮文本 */
    confirmText?: string
    /** 取消按钮文本 */
    cancelText?: string
    /** 占位提示 */
    placeholder?: string
    /** 禁用 */
    disabled?: boolean
  }>(),
  {
    modelValue: undefined,
    title: '',
    confirmText: '确定',
    cancelText: '取消',
    placeholder: '请选择',
    disabled: false,
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: string | number): void
  (e: 'change', v: string | number, option: PickerOption): void
  (e: 'confirm', v: string | number, option: PickerOption): void
  (e: 'cancel'): void
}>()

// 面板展开状态
const open = ref(false)
// 列表容器引用
const listEl = ref<HTMLElement | null>(null)
// 滚动过程中临时选中的索引（用于高亮显示，未确认前不更新 modelValue）
const tempIndex = ref(0)

// 每项高度 40px
const ITEM_HEIGHT = 40

// 当前选中项
const selectedOption = computed(() => {
  if (props.modelValue === undefined || props.modelValue === null) return null
  return props.options.find((o) => o.value === props.modelValue) ?? null
})

// trigger 显示文本
const triggerLabel = computed(() => {
  return selectedOption.value?.label ?? props.placeholder
})

// trigger 是否填充状态（影响文字颜色）
const isFilled = computed(() => !!selectedOption.value)

const triggerClasses = computed(() => {
  const list: string[] = ['picker-trigger']
  if (open.value) list.push('picker-trigger--open')
  if (props.disabled) list.push('picker-trigger--disabled')
  if (isFilled.value) list.push('picker-trigger--filled')
  return list
})

// 打开面板：初始化 tempIndex 并滚动到当前选中项
function openPanel() {
  if (props.disabled) return
  const idx = selectedOption.value
    ? Math.max(0, props.options.findIndex((o) => o.value === props.modelValue))
    : 0
  tempIndex.value = idx
  open.value = true
  // 等面板渲染后滚动到目标位置
  nextTick(() => {
    scrollToIndex(idx)
  })
}

// 关闭面板（取消）
function closePanel() {
  open.value = false
}

// 滚动到指定索引（让该项居中）
function scrollToIndex(idx: number) {
  if (!listEl.value) return
  const target = idx * ITEM_HEIGHT
  listEl.value.scrollTo({ top: target, behavior: 'auto' })
}

// 滚动时根据 scrollTop 计算当前居中项索引
function onScroll() {
  if (!listEl.value) return
  const scrollTop = listEl.value.scrollTop
  const idx = Math.round(scrollTop / ITEM_HEIGHT)
  if (idx !== tempIndex.value && idx >= 0 && idx < props.options.length) {
    tempIndex.value = idx
  }
}

// 取消
function onCancel() {
  open.value = false
  emit('cancel')
}

// 确定
function onConfirm() {
  const idx = tempIndex.value
  const opt = props.options[idx]
  if (!opt) {
    open.value = false
    return
  }
  emit('update:modelValue', opt.value)
  emit('change', opt.value, opt)
  emit('confirm', opt.value, opt)
  open.value = false
}

// 点击遮罩
function onOverlayClick() {
  onCancel()
}

// ESC 关闭
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && open.value) onCancel()
}

if (typeof window !== 'undefined') {
  window.addEventListener('keydown', onKeydown)
}

onUnmounted(() => {
  if (typeof window !== 'undefined') {
    window.removeEventListener('keydown', onKeydown)
  }
})

// 锁定 body 滚动
function lockBody() {
  if (typeof document !== 'undefined') {
    document.body.style.overflow = 'hidden'
  }
}
function unlockBody() {
  if (typeof document !== 'undefined') {
    document.body.style.overflow = ''
  }
}

watch(open, (v) => {
  if (v) lockBody()
  else unlockBody()
})

onUnmounted(() => unlockBody())

// 遮罩 fade 动画（anime.js v4 + Vue Transition JS 钩子）
const { onEnter: onOverlayEnter, onLeave: onOverlayLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    duration: 250,
    ease: 'outQuad',
  },
  leave: {
    opacity: [1, 0],
    duration: 220,
    ease: 'inOut(2)',
  },
})

// 面板从底部滑入/滑出
const { onEnter: onSheetEnter, onLeave: onSheetLeave } = useAnimeTransition({
  enter: {
    transform: ['translateY(100%)', 'translateY(0px)'],
    duration: 320,
    ease: 'out(3)',
  },
  leave: {
    transform: ['translateY(0px)', 'translateY(100%)'],
    duration: 260,
    ease: 'inOut(2)',
  },
})
</script>

<template>
  <div class="picker">
    <!-- 触发器：输入框外观，右侧 ▾ -->
    <button
      :class="triggerClasses"
      type="button"
      :disabled="disabled"
      @click="openPanel"
    >
      <span class="picker-trigger-label">{{ triggerLabel }}</span>
      <span class="picker-trigger-arrow" :class="{ 'is-open': open }">▾</span>
    </button>

    <!-- 面板：Teleport 到 body -->
    <Teleport to="body">
      <Transition :css="false" @enter="onOverlayEnter" @leave="onOverlayLeave">
        <div v-if="open" class="picker-overlay" @click="onOverlayClick"></div>
      </Transition>
      <Transition :css="false" @enter="onSheetEnter" @leave="onSheetLeave">
        <div v-if="open" class="picker-sheet" role="dialog" aria-modal="true">
          <!-- 顶部标题栏 -->
          <div class="picker-header">
            <button class="picker-btn picker-btn--cancel" type="button" @click="onCancel">
              {{ cancelText }}
            </button>
            <span class="picker-title">{{ title }}</span>
            <button class="picker-btn picker-btn--confirm" type="button" @click="onConfirm">
              {{ confirmText }}
            </button>
          </div>

          <!-- 滚轮列表 -->
          <div class="picker-wheel">
            <!-- 选中高亮条（位于列表正中） -->
            <div class="picker-highlight"></div>
            <div
              ref="listEl"
              class="picker-list"
              @scroll="onScroll"
            >
              <!-- 顶部占位：使第一项也能滚动到正中 -->
              <div class="picker-pad"></div>
              <button
                v-for="(opt, idx) in options"
                :key="opt.value"
                type="button"
                class="picker-item"
                :class="{ 'is-selected': idx === tempIndex }"
                @click="tempIndex = idx; scrollToIndex(idx)"
              >
                {{ opt.label }}
              </button>
              <!-- 底部占位 -->
              <div class="picker-pad"></div>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
