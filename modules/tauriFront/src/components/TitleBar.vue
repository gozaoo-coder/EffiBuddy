<script setup lang="ts">
/**
 * TitleBar 自定义标题栏（配合 tauri.conf decorations:false）
 *
 * 布局（2026-08 重构）：
 * - 左侧：第一个按钮点击弹出功能菜单（FeatureMenu，左栏一重定义）
 *   + 第二个按钮切换左栏二（HistoryRail）模态
 * - 中间：多页签栏（TabBar）+ 拖拽区域（data-tauri-drag-region）
 * - 右上角窗口控件：最小化 / 最大化(还原) / 关闭
 *
 * 已移除：品牌 logo 与 "EffiBuddy" 字样（用户要求去掉顶栏品牌信息）；
 * 左栏一（IconRail）已从 layout 分离为下拉功能菜单（FeatureMenu）。
 * 左栏二模态状态由 useLayoutModes 全局单例管理，跨组件实时响应并持久化 localStorage。
 *
 * 拖拽说明：titlebar-center 上的 data-tauri-drag-region 为裸属性，
 * 仅直接点击该区域自身才触发拖拽（Tauri 2 脚本行为）；TabBar 内的
 * 按钮/非空白区域不会触发窗口拖拽，可正常交互。
 *
 * 非 Tauri 环境（纯浏览器预览）自动降级：控件空操作、不报错。
 */
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import Icon from './Icon.vue'
import TabBar from './TabBar.vue'
import FeatureMenu from './FeatureMenu.vue'
import { useLayoutModes } from '../composables/useLayoutModes'
import type { RailView } from '../types'

// 模型配置模式：隐藏聊天页签栏（与主内容区切换保持一致）
withDefaults(
  defineProps<{
    modelConfigOpen?: boolean
    /** 当前激活视图（功能菜单 ✓ 高亮） */
    activeView?: RailView | ''
    /** P2P 待配对请求计数（功能菜单 P2P 项角标） */
    pendingPairCount?: number
    /** 交流池活跃条目数（功能菜单交流池项角标） */
    poolActiveCount?: number
  }>(),
  {
    modelConfigOpen: false,
    activeView: '',
    pendingPairCount: 0,
    poolActiveCount: 0,
  },
)

const emit = defineEmits<{
  (e: 'select', view: RailView): void
  (e: 'open-plugin-page', pageId: string): void
  (e: 'open-plugin-command', commandId: string): void
  (e: 'open-clawhub'): void
  (e: 'open-p2p'): void
  (e: 'open-settings'): void
  (e: 'open-asr', kind: 'asr-stream' | 'asr-upload' | 'asr-history'): void
}>()

// 功能菜单（左栏一）：由第一个按钮点击弹出
const featureMenuVisible = ref(false)
const featureMenuTrigger = ref<HTMLElement | null>(null)

const { modes, toggleRail2Mode } = useLayoutModes()

// 左栏二模态选项：expanded = 展开完整列表；hidden = 隐藏
const rail2Options = [
  { value: 'expanded', label: '展开' },
  { value: 'hidden', label: '隐藏' },
] as const

// 当前模态文案（按钮 title 提示用）
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
    <!-- 左侧：第一个按钮弹出功能菜单（左栏一）+ 左栏二模态切换 -->
    <div class="titlebar-left" data-tauri-drag-region>
      <button
        ref="featureMenuTrigger"
        type="button"
        class="mode-btn"
        :class="{ 'is-active': featureMenuVisible }"
        title="功能菜单"
        aria-label="功能菜单"
        @click="featureMenuVisible = !featureMenuVisible"
      >
        <Icon name="menu" :size="14" />
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

    <!-- 功能菜单（左栏一重定义）：dropdown menu，由第一个按钮点击弹出 -->
    <FeatureMenu
      v-model:visible="featureMenuVisible"
      :trigger-ref="featureMenuTrigger"
      :active="$props.activeView"
      :pending-pair-count="$props.pendingPairCount"
      :pool-active-count="$props.poolActiveCount"
      @select="emit('select', $event)"
      @open-plugin-page="emit('open-plugin-page', $event)"
      @open-plugin-command="emit('open-plugin-command', $event)"
      @open-clawhub="emit('open-clawhub')"
      @open-p2p="emit('open-p2p')"
      @open-settings="emit('open-settings')"
      @open-asr="emit('open-asr', $event)"
    />

    <!-- 中间：聊天多页签栏 + 拖拽区（裸 data-tauri-drag-region：仅直接点击空白处拖拽，页签可交互） -->
    <div class="titlebar-center" data-tauri-drag-region>
      <TabBar v-if="!modelConfigOpen" />
    </div>

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
  border-radius: var(--radius-sm);
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

/* 菜单打开时高亮触发按钮 */
.mode-btn.is-active {
  background: var(--card-2);
  color: var(--primary);
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
  transform: translateX(0%) translateY(-4px);
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 88px;
  padding: 6px;
  background: var(--card);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.352);
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
  transform: translateX(0%) translateY(0);
}

.tip-title {
  font-size: var(--fs-xs);
  color: var(--muted);
  padding: 2px 0;
  white-space: nowrap;
}

.tip-opt {
  font-size: var(--fs-sm);
  color: var(--text);
  padding: 3px 6px;
  border-radius: var(--radius-sm);
  text-align: left;
  white-space: nowrap;
}

.tip-opt.active {
  background: rgba(0, 0, 0, 0.08);
  color: var(--text);
  font-weight: 600;
}

/* 中间拖拽区：页签左对齐（紧贴左侧按钮组，剩余空间可拖拽） */
.titlebar-center {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  min-width: 0;
  height: 100%;
  padding-left: 8px;
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
