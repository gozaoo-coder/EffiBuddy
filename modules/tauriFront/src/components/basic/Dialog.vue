<script setup lang="ts">
/**
 * Dialog 弹出框组件（模态对话框）
 * 参考 HarmonyOS NEXT 设计规范
 *
 * 特性：
 * - 标题 / 内容（slot 优先于 content prop）
 * - 确认/取消按钮文本可定制
 * - danger 模式：确认按钮使用 danger 色
 * - align: center（垂直居中）/ bottom（贴底部，类似 action sheet）
 * - ESC 关闭、点击遮罩关闭（可配置）
 * - body 滚动锁
 */
import { ref, watch, onUnmounted, computed, useSlots } from 'vue'

const props = withDefaults(
  defineProps<{
    /** 是否显示（v-model） */
    visible?: boolean
    /** 标题 */
    title?: string
    /** 正文文本，也可用默认 slot（slot 优先） */
    content?: string
    /** 确认按钮文本，默认 "确定" */
    confirmText?: string
    /** 取消按钮文本，默认 "取消" */
    cancelText?: string
    /** 是否显示取消按钮，默认 true */
    showCancel?: boolean
    /** 是否显示确认按钮，默认 true */
    showConfirm?: boolean
    /** 点击遮罩关闭，默认 false */
    closeOnClickOverlay?: boolean
    /** ESC 关闭，默认 true */
    closeOnEsc?: boolean
    /** 危险对话框，确认按钮用 danger 色 */
    danger?: boolean
    /** 自定义宽度，默认 420px */
    width?: string
    /** 垂直对齐：center 居中 / bottom 贴底部 */
    align?: 'center' | 'bottom'
  }>(),
  {
    visible: undefined,
    title: '',
    content: '',
    confirmText: '确定',
    cancelText: '取消',
    showCancel: true,
    showConfirm: true,
    closeOnClickOverlay: false,
    closeOnEsc: true,
    danger: false,
    width: '420px',
    align: 'center',
  },
)

const emit = defineEmits<{
  (e: 'update:visible', v: boolean): void
  (e: 'confirm'): void
  (e: 'cancel'): void
  (e: 'close'): void
}>()

const slots = useSlots()

// 内部 visible 状态：支持 v-model 与非受控两种模式
const innerVisible = ref(props.visible ?? false)
watch(
  () => props.visible,
  (v) => {
    if (v !== undefined) innerVisible.value = v
  },
)

function setVisible(v: boolean) {
  innerVisible.value = v
  emit('update:visible', v)
  if (!v) emit('close')
}

// 是否使用默认 slot 作为内容
const hasSlotContent = computed(() => !!slots.default)

// body 滚动锁
function lockBodyScroll() {
  if (typeof document !== 'undefined') {
    document.body.style.overflow = 'hidden'
  }
}
function unlockBodyScroll() {
  if (typeof document !== 'undefined') {
    document.body.style.overflow = ''
  }
}

watch(innerVisible, (v) => {
  if (v) lockBodyScroll()
  else unlockBodyScroll()
})

onUnmounted(() => {
  unlockBodyScroll()
})

// ESC 键监听
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && innerVisible.value && props.closeOnEsc) {
    e.stopPropagation()
    onCancel()
  }
}
if (typeof document !== 'undefined') {
  document.addEventListener('keydown', onKeydown)
}
onUnmounted(() => {
  if (typeof document !== 'undefined') {
    document.removeEventListener('keydown', onKeydown)
  }
})

// 遮罩点击
function onOverlayClick() {
  if (props.closeOnClickOverlay) {
    onCancel()
  }
}

// 阻止对话框内部点击冒泡到遮罩
function onDialogClick(e: MouseEvent) {
  e.stopPropagation()
}

// 确认
function onConfirm() {
  emit('confirm')
  setVisible(false)
}

// 取消
function onCancel() {
  emit('cancel')
  setVisible(false)
}

// 关闭按钮
function onClose() {
  setVisible(false)
}

const hasHeader = computed(() => !!props.title)
</script>

<template>
  <Teleport to="body">
    <Transition name="dialog" appear>
      <div
        v-if="innerVisible"
        class="dialog-root"
        :class="[`dialog-root--${align}`]"
      >
        <!-- 遮罩 -->
        <div class="dialog-overlay" @click="onOverlayClick"></div>

        <!-- 对话框卡片 -->
        <div
          class="dialog"
          :class="{
            'dialog--danger': danger,
            'dialog--bottom': align === 'bottom',
          }"
          :style="{ width }"
          @click="onDialogClick"
        >
          <!-- 标题栏 + 关闭按钮 -->
          <div class="dialog-header">
            <span v-if="hasHeader" class="dialog-title">{{ title }}</span>
            <span v-else class="dialog-title-placeholder"></span>
            <button
              type="button"
              class="dialog-close"
              aria-label="关闭"
              @click="onClose"
            >×</button>
          </div>

          <!-- 内容区 -->
          <div class="dialog-body">
            <slot v-if="hasSlotContent" />
            <div v-else-if="content" class="dialog-content-text">{{ content }}</div>
          </div>

          <!-- 底部按钮区 -->
          <div
            v-if="showCancel || showConfirm"
            class="dialog-footer"
          >
            <button
              v-if="showCancel"
              type="button"
              class="dialog-btn dialog-btn--normal"
              @click="onCancel"
            >{{ cancelText }}</button>
            <button
              v-if="showConfirm"
              type="button"
              class="dialog-btn"
              :class="danger ? 'dialog-btn--danger' : 'dialog-btn--primary'"
              @click="onConfirm"
            >{{ confirmText }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
