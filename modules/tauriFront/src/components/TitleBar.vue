<script setup lang="ts">
/**
 * TitleBar 自定义标题栏（配合 tauri.conf decorations:false）
 *
 * - 左侧品牌区 + 中间区域为拖拽区域（data-tauri-drag-region）
 * - 右上角窗口控件：最小化 / 最大化(还原) / 关闭
 * - 非 Tauri 环境（纯浏览器预览）自动降级：控件空操作、不报错
 *
 * 注：模型显示已迁移至"模型配置"二级栏目（ModelSettingsRail），标题栏不再展示模型胶囊。
 */
import { ref, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import Icon from './Icon.vue'

withDefaults(
  defineProps<{
    /** 窗口标题（左侧品牌名） */
    title?: string
  }>(),
  {
    title: 'EffiBuddy',
  },
)

// 是否运行在 Tauri 环境（浏览器 dev 预览时 __TAURI_INTERNALS__ 不存在）
const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

let appWindow: ReturnType<typeof getCurrentWindow> | null = null
let unlistenResized: (() => void) | null = null
const isMaximized = ref(false)

onMounted(() => {
  if (!isTauri) return
  try {
    appWindow = getCurrentWindow()
    void appWindow.isMaximized().then((v) => (isMaximized.value = v))
    // 窗口大小变化时同步最大化状态图标
    appWindow
      .onResized(() => {
        void appWindow?.isMaximized().then((v) => (isMaximized.value = v))
      })
      .then((fn) => (unlistenResized = fn))
  } catch {
    appWindow = null
  }
})

onUnmounted(() => {
  unlistenResized?.()
})

async function minimize() {
  try {
    await appWindow?.minimize()
  } catch {
    /* 非 Tauri 环境忽略 */
  }
}

async function toggleMaximize() {
  try {
    if (await appWindow?.isMaximized()) await appWindow?.unmaximize()
    else await appWindow?.maximize()
  } catch {
    /* ignore */
  }
}

async function close() {
  try {
    await appWindow?.close()
  } catch {
    /* ignore */
  }
}
</script>

<template>
  <header class="titlebar">
    <!-- 左侧品牌：可拖拽 -->
    <div class="titlebar-left" data-tauri-drag-region>
      <span class="titlebar-logo" data-tauri-drag-region>
        <Icon name="robot" :size="17" />
      </span>
      <span class="titlebar-title" data-tauri-drag-region>{{ title }}</span>
    </div>

    <!-- 中间拖拽区（不再显示模型胶囊） -->
    <div class="titlebar-center" data-tauri-drag-region></div>

    <!-- 右上角窗口控件：非拖拽区域 -->
    <div class="titlebar-controls">
      <button
        type="button"
        class="win-btn"
        title="最小化"
        aria-label="最小化"
        @click="minimize"
      >
        <svg width="11" height="11" viewBox="0 0 11 11" fill="none">
          <path d="M0.5 5.5H10.5" stroke="currentColor" stroke-width="1.2" />
        </svg>
      </button>

      <button
        type="button"
        class="win-btn"
        :title="isMaximized ? '还原' : '最大化'"
        :aria-label="isMaximized ? '还原' : '最大化'"
        @click="toggleMaximize"
      >
        <svg v-if="!isMaximized" width="11" height="11" viewBox="0 0 11 11" fill="none">
          <rect x="0.5" y="0.5" width="10" height="10" rx="1.2" stroke="currentColor" stroke-width="1.2" />
        </svg>
        <svg v-else width="12" height="12" viewBox="0 0 12 12" fill="none">
          <rect x="0.5" y="2.5" width="9" height="9" rx="1" stroke="currentColor" stroke-width="1.1" />
          <path d="M3 2.5V1.5C3 0.9 3.5 0.5 4 0.5H10.5C11 0.5 11.5 1 11.5 1.5V8C11.5 8.5 11 9 10.5 9H9.5" stroke="currentColor" stroke-width="1.1" />
        </svg>
      </button>

      <button
        type="button"
        class="win-btn win-btn--close"
        title="关闭"
        aria-label="关闭"
        @click="close"
      >
        <svg width="11" height="11" viewBox="0 0 11 11" fill="none">
          <path d="M1 1L10 10M10 1L1 10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
        </svg>
      </button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  align-items: center;
  height: 44px;
  flex-shrink: 0;
  background: var(--bg-2);
  border-bottom: 1px solid var(--border);
  user-select: none;
}

/* 左侧品牌 */
.titlebar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 14px;
  min-width: 0;
  height: 100%;
}

.titlebar-logo {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--radius-sm);
  background: linear-gradient(135deg, var(--primary), var(--primary-dim));
  color: #fff;
  flex-shrink: 0;
  box-shadow: var(--shadow-sm);
}

.titlebar-title {
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.3px;
  color: var(--text);
  white-space: nowrap;
}

/* 中间拖拽区（不再显示模型胶囊） */
.titlebar-center {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  height: 100%;
}

/* 右上角窗口控件 */
.titlebar-controls {
  display: flex;
  align-items: stretch;
  height: 100%;
  flex-shrink: 0;
}

.win-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 46px;
  height: 100%;
  padding: 0;
  background: transparent;
  border: none;
  color: var(--text);
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.win-btn:hover {
  background: var(--card-2);
  color: var(--text);
}

.win-btn:active {
  background: var(--border);
}

/* 关闭按钮：Windows 风格红色 hover */
.win-btn--close:hover {
  background: #e81123;
  color: #fff;
}

.win-btn--close:active {
  background: #b00e1a;
}
</style>
