<script setup lang="ts">
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  BindSheet,
  Button,
  Icon,
  RadioGroup,
  Radio,
  Slider,
  Dialog,
  useToast,
} from './basic'
import { useTheme } from '../composables/useTheme'
import { useAnimeTransition } from '../composables/useAnimeTransition'
import PinnedMemoryPanel from './PinnedMemoryPanel.vue'
import type { ThemeMode, ConversationMeta } from '../types'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const { toast } = useToast()
const { themeMode, resolvedTheme, setTheme } = useTheme()

// 当前选中的分类
type SettingsTab = 'appearance' | 'memory' | 'data' | 'about'
const activeTab = ref<SettingsTab>('appearance')

const tabs: { key: SettingsTab; icon: string; label: string }[] = [
  { key: 'appearance', icon: 'palette', label: '外观' },
  { key: 'memory', icon: 'pin', label: '永久记忆' },
  { key: 'data', icon: 'cloud', label: '数据管理' },
  { key: 'about', icon: 'info', label: '关于' },
]

// 字体大小（本地占位，未来可持久化）
const fontSize = ref(14)
function onFontSizeChange(v: number) {
  fontSize.value = v
  document.documentElement.style.fontSize = `${v}px`
}

// 分类切换淡入动画
const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [10, 0],
    duration: 300,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateY: [0, -8],
    duration: 200,
    ease: 'inOut(2)',
  },
})

function onThemeChange(v: unknown) {
  setTheme(v as ThemeMode)
}

// ---------- 数据管理 ----------
const clearDataDialog = ref(false)
const clearConvDialog = ref(false)
const clearing = ref(false)

const resolvedThemeLabel = computed(() =>
  resolvedTheme.value === 'dark' ? '暗色' : '亮色',
)

async function clearAllData() {
  clearing.value = true
  try {
    let usedFallback = false
    try {
      await invoke('clear_all_data')
    } catch {
      usedFallback = true
      const convs = await invoke<ConversationMeta[]>('list_conversations')
      for (const c of convs) {
        try {
          await invoke('delete_conversation', { id: c.id })
        } catch {
          /* 忽略单条失败 */
        }
      }
    }
    toast({
      content: usedFallback ? '已清除所有会话数据，建议重启应用' : '已清除所有应用数据',
      type: 'success',
    })
  } catch (e) {
    toast({ content: `清除失败：${e}`, type: 'error' })
  } finally {
    clearing.value = false
    clearDataDialog.value = false
  }
}

async function clearConversations() {
  clearing.value = true
  try {
    const convs = await invoke<ConversationMeta[]>('list_conversations')
    for (const c of convs) {
      try {
        await invoke('delete_conversation', { id: c.id })
      } catch {
        /* 忽略单条失败 */
      }
    }
    toast({ content: `已清除 ${convs.length} 条会话`, type: 'success' })
  } catch (e) {
    toast({ content: `清除失败：${e}`, type: 'error' })
  } finally {
    clearing.value = false
    clearConvDialog.value = false
  }
}

function exportData() {
  toast({ content: '数据导出功能即将上线', type: 'info' })
}

// ---------- 关于 ----------
const APP_NAME = 'EffiBuddy'
const APP_VERSION = '0.1.0'
const GITHUB_URL = 'https://github.com/EffiSuite/EffiBuddy'

function openGithub() {
  if (typeof window !== 'undefined') {
    window.open(GITHUB_URL, '_blank', 'noopener')
  }
}

function onClose() {
  emit('close')
}
</script>

<template>
  <BindSheet
    :visible="props.open"
    side="right"
    width="640px"
    title="设置"
    @close="onClose"
  >
    <div class="settings-shell">
      <!-- 左侧分类导航 -->
      <nav class="settings-nav">
        <button
          v-for="t in tabs"
          :key="t.key"
          type="button"
          class="nav-item"
          :class="{ active: activeTab === t.key }"
          @click="activeTab = t.key"
        >
          <span class="nav-icon"><Icon :name="t.icon" :size="20" /></span>
          <span class="nav-label">{{ t.label }}</span>
        </button>
      </nav>

      <!-- 右侧内容区 -->
      <div class="settings-content">
        <Transition :css="false" @enter="onEnter" @leave="onLeave" mode="out-in">
          <!-- 外观页 -->
          <section v-if="activeTab === 'appearance'" key="appearance" class="page">
            <header class="page-head">
              <h2 class="page-title">外观</h2>
              <p class="page-sub">定制 EffiBuddy 的视觉风格</p>
            </header>

            <div class="card">
              <div class="card-head">
                <span class="card-title">主题模式</span>
                <span class="card-badge">{{ resolvedThemeLabel }}</span>
              </div>
              <RadioGroup
                :model-value="themeMode"
                name="theme-mode"
                @change="onThemeChange"
              >
                <label class="radio-row">
                  <Radio value="system" />
                  <span class="radio-row-text">
                    <span class="radio-row-label">跟随系统</span>
                    <span class="radio-row-hint">自动匹配操作系统主题</span>
                  </span>
                  <span class="radio-row-glyph"><Icon name="auto" :size="22" /></span>
                </label>
                <label class="radio-row">
                  <Radio value="light" />
                  <span class="radio-row-text">
                    <span class="radio-row-label">亮色</span>
                    <span class="radio-row-hint">明亮、清爽的日间模式</span>
                  </span>
                  <span class="radio-row-glyph"><Icon name="sun" :size="22" /></span>
                </label>
                <label class="radio-row">
                  <Radio value="dark" />
                  <span class="radio-row-text">
                    <span class="radio-row-label">暗色</span>
                    <span class="radio-row-hint">护眼、沉浸的夜间模式</span>
                  </span>
                  <span class="radio-row-glyph"><Icon name="moon" :size="22" /></span>
                </label>
              </RadioGroup>
            </div>

            <div class="card">
              <div class="card-head">
                <span class="card-title">字体大小</span>
                <span class="card-badge">{{ fontSize }}px</span>
              </div>
              <Slider
                :model-value="fontSize"
                :min="12"
                :max="18"
                :step="1"
                show-value
                @change="onFontSizeChange"
              />
              <p class="card-hint">调整界面正文字号（12–18px）</p>
            </div>
          </section>

          <!-- 永久记忆页 -->
          <PinnedMemoryPanel v-else-if="activeTab === 'memory'" key="memory" :open="props.open" />

          <!-- 数据管理页 -->
          <section v-else-if="activeTab === 'data'" key="data" class="page">
            <header class="page-head">
              <h2 class="page-title">数据管理</h2>
              <p class="page-sub">管理本地会话与应用数据</p>
            </header>

            <div class="action-card action-card--danger">
              <div class="action-card-text">
                <span class="action-card-title">清除所有会话</span>
                <span class="action-card-hint">删除全部对话记录，不可恢复</span>
              </div>
              <Button variant="danger" size="sm" @click="clearConvDialog = true">
                清除
              </Button>
            </div>

            <div class="action-card action-card--danger">
              <div class="action-card-text">
                <span class="action-card-title">清除所有应用数据</span>
                <span class="action-card-hint">重置所有本地数据，建议重启应用</span>
              </div>
              <Button variant="danger" size="sm" @click="clearDataDialog = true">
                清除
              </Button>
            </div>

            <div class="action-card">
              <div class="action-card-text">
                <span class="action-card-title">导出数据</span>
                <span class="action-card-hint">将对话与配置导出为文件</span>
              </div>
              <Button variant="normal" size="sm" @click="exportData">
                导出
              </Button>
            </div>
          </section>

          <!-- 关于页 -->
          <section v-else key="about" class="page">
            <header class="page-head">
              <h2 class="page-title">关于</h2>
              <p class="page-sub">了解 EffiBuddy</p>
            </header>

            <div class="about-hero">
              <div class="about-mark">EB</div>
              <div class="about-id">
                <div class="about-name">{{ APP_NAME }}</div>
                <div class="about-version">版本 {{ APP_VERSION }}</div>
              </div>
            </div>

            <p class="about-desc">
              EffiBuddy 是一款高效的 AI 助手桌面应用，提供流畅的对话体验、
              设备协同与可插拔的模型配置能力。
            </p>

            <div class="card">
              <div class="card-head">
                <span class="card-title">技术栈</span>
              </div>
              <div class="tech-grid">
                <span class="tech-chip">Vue 3.5</span>
                <span class="tech-chip">Tauri 2</span>
                <span class="tech-chip">Rust</span>
                <span class="tech-chip">rig</span>
                <span class="tech-chip">anime.js v4</span>
              </div>
            </div>

            <div class="card about-links">
              <button type="button" class="link-row" @click="openGithub">
                <span class="link-glyph"><Icon name="book" :size="18" /></span>
                <span class="link-text">GitHub 仓库</span>
                <span class="link-arrow"><Icon name="external-link" :size="14" /></span>
              </button>
              <div class="link-row link-row--static">
                <span class="link-glyph"><Icon name="book" :size="18" /></span>
                <span class="link-text">开源许可 (MIT)</span>
              </div>
            </div>
          </section>
        </Transition>
      </div>
    </div>

    <!-- 清除所有会话确认 -->
    <Dialog
      :visible="clearConvDialog"
      title="清除所有会话？"
      content="此操作将删除全部对话记录，且无法撤销。"
      confirm-text="清除"
      danger
      @confirm="clearConversations"
      @cancel="clearConvDialog = false"
    />

    <!-- 清除所有应用数据确认 -->
    <Dialog
      :visible="clearDataDialog"
      title="清除所有应用数据？"
      content="将重置所有本地数据（含会话与配置缓存），建议清除后重启应用。此操作不可撤销。"
      confirm-text="全部清除"
      danger
      @confirm="clearAllData"
      @cancel="clearDataDialog = false"
    />
  </BindSheet>
</template>

<style scoped>
.settings-shell {
  display: flex;
  height: 100%;
  min-height: 0;
}

/* ---------- 左侧导航 ---------- */
.settings-nav {
  width: 180px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 16px 12px;
  border-right: 1px solid var(--border);
  background: var(--bg-2);
  overflow-y: auto;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  font-family: inherit;
  font-size: var(--fs-base);
  font-weight: 500;
  line-height: 1;
  text-align: left;
  cursor: pointer;
  user-select: none;
  outline: none;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
}

.nav-item:hover {
  background: var(--card);
  color: var(--text);
}

.nav-item.active {
  background: var(--card-2);
  color: var(--primary);
  border-color: var(--primary);
}

.nav-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  font-size: 15px;
  line-height: 1;
  flex-shrink: 0;
}

.nav-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---------- 右侧内容区 ---------- */
.settings-content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 24px 28px 32px;
}

.page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.page-head {
  margin-bottom: 4px;
}

.page-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text);
}

.page-sub {
  margin: 4px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
}

/* ---------- 通用卡片 ---------- */
.card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.card-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.card-badge {
  font-size: var(--fs-xs);
  color: var(--primary);
  padding: 2px 8px;
  border: 1px solid var(--primary);
  border-radius: var(--radius-full);
  line-height: 1.4;
}

.card-hint {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 主题单选行 ---------- */
.radio-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-2);
  cursor: pointer;
  transition: border-color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard);
}

.radio-row:hover {
  border-color: var(--primary);
}

.radio-row-text {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.radio-row-label {
  font-size: var(--fs-base);
  font-weight: 500;
  color: var(--text);
}

.radio-row-hint {
  font-size: var(--fs-xs);
  color: var(--muted);
}

.radio-row-glyph {
  font-size: var(--fs-lg);
  color: var(--muted);
  line-height: 1;
  flex-shrink: 0;
}

/* ---------- 数据管理操作卡 ---------- */
.action-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 16px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.action-card--danger {
  border-color: rgba(255, 92, 92, 0.35);
}

.action-card--danger:hover {
  border-color: var(--danger);
}

.action-card-text {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.action-card-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.action-card-hint {
  font-size: var(--fs-sm);
  color: var(--muted);
}

/* ---------- 关于页 ---------- */
.about-hero {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 20px;
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.12), rgba(74, 126, 255, 0.02));
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.about-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  border-radius: var(--radius);
  background: var(--primary);
  color: #fff;
  font-size: 22px;
  font-weight: 700;
  letter-spacing: 1px;
  flex-shrink: 0;
}

.about-id {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.about-name {
  font-size: var(--fs-xl);
  font-weight: 700;
  color: var(--text);
}

.about-version {
  font-size: var(--fs-sm);
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.about-desc {
  margin: 0;
  font-size: var(--fs-base);
  line-height: 1.6;
  color: var(--text);
}

.tech-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.tech-chip {
  padding: 4px 12px;
  font-size: var(--fs-sm);
  color: var(--text);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
}

.about-links {
  gap: 4px;
}

.link-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-size: var(--fs-base);
  text-align: left;
  cursor: pointer;
  outline: none;
  transition: background var(--duration-fast) var(--ease-standard);
}

.link-row:hover {
  background: var(--card-2);
}

.link-row--static {
  cursor: default;
}

.link-row--static:hover {
  background: transparent;
}

.link-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  font-size: 15px;
  color: var(--muted);
  flex-shrink: 0;
}

.link-text {
  flex: 1;
}

.link-arrow {
  font-size: var(--fs-sm);
  color: var(--muted);
}
</style>
