<script setup lang="ts">
/**
 * TitleBar 自定义标题栏（配合 tauri.conf decorations:false）
 *
 * 布局（2026-08 重构）：
 * - 左侧：左栏一（IconRail）模态切换 + 左栏二（HistoryRail）模态切换按钮
 * - 中间：拖拽区域（data-tauri-drag-region）
 * - 右上角窗口控件：最小化 / 最大化(还原) / 关闭
 *
 * 已移除：品牌 logo 与 "EffiBuddy" 字样（用户要求去掉顶栏品牌信息）。
 * 模态状态由 useLayoutModes 全局单例管理，跨组件实时响应并持久化 localStorage。
 *
 * 非 Tauri 环境（纯浏览器预览）自动降级：控件空操作、不报错。
 */
import { ref, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import Icon from './Icon.vue'
import { SegmentedButton } from './basic'
import { useLayoutModes } from '../composables/useLayoutModes'

const { modes, toggleRail1Mode, toggleRail2Mode } = useLayoutModes()

// 左栏一模态选项：icon = 纯图标窄栏；icon-text = 图标+文字宽栏
const rail1Options = [
  { value: 'icon', label: '图标' },
  { value: 'icon-text', label: '图标+文字' },
]
// 左栏二模态选项：expanded = 展开完整列表；collapsed = 收起窄条
const rail2Options = [
  { value: 'expanded', label: '展开' },
  { value: 'collapsed', label: '收起' },
]

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
    <!-- 左侧：左栏一 / 左栏二 模态切换（可拖拽区域，按钮本身不响应拖拽） -->
    <div class="titlebar-left" data-tauri-drag-region>
      <div class="mode-group" title="左栏一模态：纯图标 / 图标+文字">
        <span class="mode-label">
          <Icon name="menu" :size="13" />
        </span>
        <SegmentedButton
          :model-value="modes.rail1"
          :options="rail1Options"
          size="sm"
          @update:model-value="toggleRail1Mode"
        />
      </div>

      <div class="mode-sep" />

      <div class="mode-group" title="左栏二模态：展开 / 收起">
        <span class="mode-label">
          <Icon name="folder" :size="13" />
        </span>
        <SegmentedButton
          :model-value="modes.rail2"
          :options="rail2Options"
          size="sm"
          @update:model-value="toggleRail2Mode"
        />
      </div>
    </div>

    <!-- 中间拖拽区 -->
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

/* 左侧：模态切换按钮组 */
.titlebar-left {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 10px;
  min-width: 0;
  height: 100%;
}

.mode-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.mode-label {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--muted);
  flex-shrink: 0;
}

.mode-sep {
  width: 1px;
  height: 18px;
  background: var(--border);
  flex-shrink: 0;
}

/* 让分段按钮在深色顶栏上保持紧凑观感 */
.titlebar-left :deep(.segmented) {
  background: var(--card);
  border-color: var(--border);
}

.titlebar-left :deep(.segmented-item) {
  color: var(--muted);
}

.titlebar-left :deep(.segmented-item.is-selected) {
  background: var(--primary);
  color: #fff;
}

/* 中间拖拽区 */
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
