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
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import Icon from './Icon.vue'
import { useLayoutModes } from '../composables/useLayoutModes'

const { modes, toggleRail1Mode, toggleRail2Mode } = useLayoutModes()

// 左栏一模态选项：icon = 纯图标窄栏；icon-text = 图标+文字宽栏
const rail1Options = [
  { value: 'icon', label: '图标' },
  { value: 'icon-text', label: '图标+文字' },
] as const
// 左栏二模态选项：expanded = 展开完整列表；collapsed = 收起窄条
const rail2Options = [
  { value: 'expanded', label: '展开' },
  { value: 'collapsed', label: '收起' },
] as const

// 当前模态文案（按钮 title 提示用）
const rail1Label = computed(
  () => rail1Options.find((o) => o.value === modes.value.rail1)?.label ?? '',
)
const rail2Label = computed(
  () => rail2Options.find((o) => o.value === modes.value.rail2)?.label ?? '',
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
    <!-- 左侧：左栏一 / 左栏二 模态切换（单按钮：hover 浮出选项，click 直接切换） -->
    <div class="titlebar-left" data-tauri-drag-region>
      <button
        type="button"
        class="mode-btn"
        :title="`左栏一：${rail1Label}（点击切换）`"
        @click="toggleRail1Mode"
      >
        <Icon name="menu" :size="14" />
        <span class="mode-tip" aria-hidden="true">
          <span class="tip-title">左栏一</span>
          <span
            v-for="opt in rail1Options"
            :key="opt.value"
            class="tip-opt"
            :class="{ active: modes.rail1 === opt.value }"
          >{{ opt.label }}</span>
        </span>
      </button>

      <div class="mode-sep" />

      <button
        type="button"
        class="mode-btn"
        :title="`左栏二：${rail2Label}（点击切换）`"
        @click="toggleRail2Mode"
      >
        <Icon name="folder" :size="14" />
        <span class="mode-tip" aria-hidden="true">
          <span class="tip-title">左栏二</span>
          <span
            v-for="opt in rail2Options"
            :key="opt.value"
            class="tip-opt"
            :class="{ active: modes.rail2 === opt.value }"
          >{{ opt.label }}</span>
        </span>
      </button>
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
  background: var(--bg-chrome);
  border-bottom: 1px solid var(--border-strong);
  user-select: none;
}

/* 左侧：模态切换按钮组（紧凑布局：小 padding / 窄 gap） */
.titlebar-left {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 6px 0 8px;
  min-width: 0;
  height: 100%;
}

/* 模态切换按钮：与顶栏融为一体的幽灵按钮 */
.mode-btn {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  background: transparent;
  border: none;
  border-radius: var(--radius-xs);
  color: var(--muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.mode-btn:hover {
  background: var(--card-2);
  color: var(--text);
}

.mode-btn:active {
  background: var(--border);
}

.mode-sep {
  width: 1px;
  height: 16px;
  background: var(--border-strong);
  flex-shrink: 0;
  margin: 0 2px;
}

/* hover 浮出的模态选项提示面板 */
.mode-tip {
  position: absolute;
  top: calc(100% + 6px);
  left: 50%;
  transform: translateX(-50%) translateY(-4px);
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 88px;
  padding: 6px;
  background: var(--card);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  z-index: 100;
  transition: opacity var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard),
    visibility var(--duration-fast);
}

.mode-btn:hover .mode-tip {
  opacity: 1;
  visibility: visible;
  transform: translateX(-50%) translateY(0);
}

.tip-title {
  font-size: var(--fs-xs);
  color: var(--muted);
  padding: 2px 6px;
  white-space: nowrap;
}

.tip-opt {
  font-size: var(--fs-sm);
  color: var(--text);
  padding: 3px 6px;
  border-radius: var(--radius-xs);
  text-align: left;
  white-space: nowrap;
}

.tip-opt.active {
  color: var(--primary);
  font-weight: 600;
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
