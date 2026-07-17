<script setup lang="ts">
/**
 * Dropdown 下拉选择组件
 * 点击 trigger 展开选项浮层，浮层 Teleport 到 body
 * 支持 searchable 搜索框、自定义宽度、外部点击关闭、淡入下移动效
 */
import { ref, computed, watch, nextTick, onUnmounted } from 'vue'
import { useAnimeTransition } from '../../composables/useAnimeTransition'

export interface DropdownOption {
  label: string
  value: string | number
  /** 图标（可选，emoji 或字符） */
  icon?: string
}

export type DropdownSize = 'sm' | 'md' | 'lg'

const props = withDefaults(
  defineProps<{
    /** 选项列表 */
    options: DropdownOption[]
    /** 当前选中值（v-model） */
    modelValue?: string | number
    /** 占位提示 */
    placeholder?: string
    /** 是否带搜索框 */
    searchable?: boolean
    /** 尺寸：sm 28 / md 36 / lg 44 */
    size?: DropdownSize
    /** 禁用 */
    disabled?: boolean
    /** 自定义宽度，默认 100% */
    width?: string
  }>(),
  {
    modelValue: undefined,
    placeholder: '请选择',
    searchable: false,
    size: 'md',
    disabled: false,
    width: '100%',
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: string | number): void
  (e: 'change', v: string | number, option: DropdownOption): void
}>()

// 浮层展开状态
const open = ref(false)
// 搜索关键字
const query = ref('')
// trigger 元素引用
const triggerEl = ref<HTMLElement | null>(null)
// 浮层元素引用
const panelEl = ref<HTMLElement | null>(null)
// 浮层定位样式
const panelStyle = ref<Record<string, string>>({})

// 浮层进入/离开动画：淡入 + 从上方下移 8px 进入（保持原 CSS translateY(-8px) 行为）
const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    transform: ['translateY(-8px)', 'translateY(0px)'],
    duration: 180,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    transform: ['translateY(0px)', 'translateY(-8px)'],
    duration: 150,
    ease: 'inOut(2)',
  },
})

// 当前选中项
const selectedOption = computed(() => {
  if (props.modelValue === undefined || props.modelValue === null) return null
  return props.options.find((o) => o.value === props.modelValue) ?? null
})

// 显示在 trigger 上的文本
const triggerLabel = computed(() => {
  if (selectedOption.value) return selectedOption.value.label
  return props.placeholder
})

// 经过搜索过滤后的选项列表
const filteredOptions = computed(() => {
  const q = query.value.trim().toLowerCase()
  if (!q) return props.options
  return props.options.filter((o) => o.label.toLowerCase().includes(q))
})

// trigger class 组合
const triggerClasses = computed(() => {
  const list: string[] = ['dropdown-trigger']
  list.push(`dropdown-trigger--${props.size}`)
  if (open.value) list.push('dropdown-trigger--open')
  if (props.disabled) list.push('dropdown-trigger--disabled')
  if (selectedOption.value) list.push('dropdown-trigger--filled')
  return list
})

// 切换浮层显示
function toggleOpen() {
  if (props.disabled) return
  open.value = !open.value
  if (open.value) {
    // 打开时清空搜索词
    query.value = ''
    nextTick(() => updatePosition())
  }
}

// 计算浮层定位（基于 trigger 的 getBoundingClientRect）
async function updatePosition() {
  if (!open.value || !triggerEl.value) return
  await nextTick()
  if (!triggerEl.value) return
  const rect = triggerEl.value.getBoundingClientRect()
  const vw = window.innerWidth
  const vh = window.innerHeight

  // 默认显示在 trigger 下方
  let top = rect.bottom + 4
  let left = rect.left

  // 等浮层渲染后获取高度，避免超出屏幕底部
  await nextTick()
  const panelHeight = panelEl.value?.offsetHeight ?? 280
  if (top + panelHeight > vh - 8) {
    // 放在 trigger 上方
    top = rect.top - panelHeight - 4
    if (top < 8) top = 8
  }
  // 边界适配
  if (left < 8) left = 8
  if (left + rect.width > vw - 8) left = vw - rect.width - 8

  panelStyle.value = {
    top: `${top + window.scrollY}px`,
    left: `${left + window.scrollX}px`,
    minWidth: `${rect.width}px`,
  }
}

// 选项点击
function onSelect(option: DropdownOption) {
  emit('update:modelValue', option.value)
  emit('change', option.value, option)
  open.value = false
}

// 点击外部关闭
function onDocumentClick(e: MouseEvent) {
  if (!open.value) return
  const target = e.target as Node
  if (triggerEl.value?.contains(target)) return
  if (panelEl.value?.contains(target)) return
  open.value = false
}

// 滚动/缩放时重新定位
function onScroll() {
  if (open.value) updatePosition()
}

if (typeof document !== 'undefined') {
  document.addEventListener('click', onDocumentClick)
}
if (typeof window !== 'undefined') {
  window.addEventListener('scroll', onScroll, true)
  window.addEventListener('resize', onScroll)
}

onUnmounted(() => {
  if (typeof document !== 'undefined') {
    document.removeEventListener('click', onDocumentClick)
  }
  if (typeof window !== 'undefined') {
    window.removeEventListener('scroll', onScroll, true)
    window.removeEventListener('resize', onScroll)
  }
})

// 阻止搜索框点击事件冒泡到 document 防止误关闭
function onPanelClick(e: MouseEvent) {
  e.stopPropagation()
}

function onTriggerClick(e: MouseEvent) {
  e.stopPropagation()
  toggleOpen()
}

// 搜索框键盘事件：阻止冒泡，避免影响外部
function onSearchKeydown(e: KeyboardEvent) {
  e.stopPropagation()
}
</script>

<template>
  <div class="dropdown" :style="{ width }">
    <!-- 触发按钮 -->
    <button
      ref="triggerEl"
      :class="triggerClasses"
      :disabled="disabled"
      type="button"
      @click="onTriggerClick"
    >
      <span v-if="selectedOption?.icon" class="dropdown-trigger-icon">
        {{ selectedOption.icon }}
      </span>
      <span class="dropdown-trigger-label">{{ triggerLabel }}</span>
      <span class="dropdown-trigger-arrow" :class="{ 'is-open': open }">▾</span>
    </button>

    <!-- 浮层：Teleport 到 body -->
    <Teleport to="body">
      <Transition :css="false" @enter="onEnter" @leave="onLeave" appear>
        <div
          v-if="open"
          ref="panelEl"
          class="dropdown-panel"
          :style="panelStyle"
          @click="onPanelClick"
        >
          <!-- 搜索框 -->
          <div v-if="searchable" class="dropdown-search">
            <input
              v-model="query"
              class="dropdown-search-input"
              type="text"
              placeholder="搜索..."
              autocomplete="off"
              @keydown="onSearchKeydown"
            />
          </div>

          <!-- 选项列表 -->
          <div class="dropdown-list">
            <div v-if="filteredOptions.length === 0" class="dropdown-empty">
              无匹配项
            </div>
            <button
              v-for="opt in filteredOptions"
              :key="opt.value"
              type="button"
              class="dropdown-item"
              :class="{ 'is-selected': opt.value === modelValue }"
              @click="onSelect(opt)"
            >
              <span v-if="opt.icon" class="dropdown-item-icon">{{ opt.icon }}</span>
              <span class="dropdown-item-label">{{ opt.label }}</span>
              <span v-if="opt.value === modelValue" class="dropdown-item-check">✓</span>
            </button>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
