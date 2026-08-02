<script setup lang="ts">
/**
 * ImagePreview —— 图片全屏预览(Teleport 到 body)
 *
 * 支持滚轮缩放、拖拽平移、按钮旋转、双击复位、Esc 关闭。
 * window keydown 监听在本组件注册/卸载。
 * 样式为非 scoped:Teleport 到 body 后 scoped 属性不生效。
 */
import { inject, onMounted, onUnmounted } from 'vue'
import { Icon } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const {
  previewState,
  closeImagePreview,
  previewZoomIn,
  previewZoomOut,
  previewRotate,
  resetPreviewTransform,
  onPreviewWheel,
  onPreviewPointerDown,
  onPreviewPointerMove,
  onPreviewPointerUp,
  onPreviewDblClick,
  onPreviewKeydown,
} = store.preview

onMounted(() => {
  window.addEventListener('keydown', onPreviewKeydown)
})
onUnmounted(() => {
  window.removeEventListener('keydown', onPreviewKeydown)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="img-preview-fade">
      <div
        v-if="previewState.visible"
        class="img-preview-overlay"
        @click="closeImagePreview"
        @wheel="onPreviewWheel"
      >
        <img
          :src="previewState.url"
          :alt="previewState.name"
          class="img-preview-img"
          :style="{
            transform: `translate(${previewState.tx}px, ${previewState.ty}px) scale(${previewState.scale}) rotate(${previewState.rotate}deg)`,
          }"
          @click.stop
          @pointerdown="onPreviewPointerDown"
          @pointermove="onPreviewPointerMove"
          @pointerup="onPreviewPointerUp"
          @pointercancel="onPreviewPointerUp"
          @dblclick="onPreviewDblClick"
          draggable="false"
        />
        <div class="img-preview-name">{{ previewState.name }}</div>

        <!-- 工具栏:放大 / 缩小 / 旋转 / 复位 / 关闭 -->
        <div class="img-preview-toolbar" @click.stop>
          <button type="button" class="img-preview-tool-btn" title="放大（+）" @click="previewZoomIn">
            <Icon name="plus" :size="20" />
          </button>
          <span class="img-preview-zoom-label">{{ Math.round(previewState.scale * 100) }}%</span>
          <button type="button" class="img-preview-tool-btn" title="缩小（-）" @click="previewZoomOut">
            <Icon name="minus" :size="20" />
          </button>
          <span class="img-preview-tool-divider"></span>
          <button type="button" class="img-preview-tool-btn" title="旋转 90°（R）" @click="previewRotate">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" fill-rule="evenodd"><path d="M4 12C4 7.6 7.6 4 12 4C14.5 4 16.7 5.1 18.2 6.9M20 4V9H15M20 12C20 16.4 16.4 20 12 20C9.5 20 7.3 18.9 5.8 17.1M4 20V15H9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
          </button>
          <button type="button" class="img-preview-tool-btn" title="复位（0）" @click="resetPreviewTransform">
            <Icon name="refresh" :size="20" />
          </button>
          <span class="img-preview-tool-divider"></span>
          <button
            type="button"
            class="img-preview-tool-btn img-preview-tool-btn--close"
            title="关闭（Esc）"
            @click="closeImagePreview"
          >
            <Icon name="close" :size="20" />
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style>
.img-preview-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.82);
  backdrop-filter: blur(6px);
  user-select: none;
  cursor: zoom-out;
}

.img-preview-img {
  max-width: 88vw;
  max-height: 82vh;
  object-fit: contain;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
  cursor: grab;
  touch-action: none;
}

.img-preview-img:active {
  cursor: grabbing;
}

.img-preview-name {
  position: absolute;
  top: 16px;
  left: 50%;
  transform: translateX(-50%);
  font-size: 13px;
  color: rgba(255, 255, 255, 0.85);
  max-width: 60vw;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-shadow: 0 1px 4px rgba(0, 0, 0, 0.5);
}

/* 工具栏:放大 / 缩小 / 旋转 / 复位 / 关闭 */
.img-preview-toolbar {
  position: absolute;
  bottom: 20px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 8px 12px;
  background: rgba(30, 30, 30, 0.85);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: var(--radius-full);
  backdrop-filter: blur(8px);
  cursor: default;
}

.img-preview-tool-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: rgba(255, 255, 255, 0.85);
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.img-preview-tool-btn:hover {
  background: rgba(255, 255, 255, 0.14);
  color: #fff;
}

.img-preview-tool-btn--close:hover {
  background: rgba(255, 80, 80, 0.35);
  color: #fff;
}

.img-preview-zoom-label {
  min-width: 48px;
  text-align: center;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.75);
  font-variant-numeric: tabular-nums;
}

.img-preview-tool-divider {
  width: 1px;
  height: 20px;
  background: rgba(255, 255, 255, 0.2);
  margin: 0 4px;
}

.img-preview-fade-enter-active,
.img-preview-fade-leave-active {
  transition: opacity 0.18s ease;
}

.img-preview-fade-enter-from,
.img-preview-fade-leave-to {
  opacity: 0;
}
</style>
