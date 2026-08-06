<script setup lang="ts">
/**
 * WidgetDesktopTab —— 桌面小组件页签
 *
 * 功能菜单「桌面小组件」入口打开的页签容器：
 * - 渲染 WidgetDesktop（网格 / 堆叠双布局）
 * - 内置演示小组件：覆盖全部 WidgetSize 尺寸枚举（tiny → full）
 * - 布局偏好持久化 localStorage；切换页签后由 TabContent KeepAlive 缓存状态
 */
import { onMounted, ref } from 'vue'
import WidgetDesktop, { type DesktopLayout } from '../widget/WidgetDesktop.vue'
import { WidgetSize, WIDGET_SIZE_MAP } from '../../types/widget'
import type { WidgetItem } from '../../types/widget'
import type { TabItem } from '../../types'

defineOptions({ name: 'WidgetDesktopTab' })

defineProps<{
  tab: TabItem
}>()

const STORAGE_KEY = 'effisuite:widget-desktop-layout'
const layout = ref<DesktopLayout>('grid')

onMounted(() => {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved === 'grid' || saved === 'stack' || saved === 'free') layout.value = saved
  } catch {
    /* ignore */
  }
})

function onLayoutChange(v: DesktopLayout) {
  layout.value = v
  try {
    localStorage.setItem(STORAGE_KEY, v)
  } catch {
    /* ignore */
  }
}

/** 演示小组件：每种尺寸枚举一档（展示尺寸差异与堆叠效果） */
const demoWidgets: WidgetItem[] = [
  {
    id: 'demo-tiny',
    title: '极小',
    size: WidgetSize.TINY,
    position: { col: 1, row: 1 },
    component: 'DemoTiny',
    props: { emoji: '📌', caption: '快捷笔记' },
  },
  {
    id: 'demo-small',
    title: '小',
    size: WidgetSize.SMALL,
    position: { col: 2, row: 1 },
    component: 'DemoSmall',
    props: { emoji: '📊', caption: '今日数据' },
  },
  {
    id: 'demo-medium',
    title: '中',
    size: WidgetSize.MEDIUM,
    position: { col: 3, row: 1 },
    component: 'DemoMedium',
    props: { emoji: '🧭', caption: '状态总览' },
  },
  {
    id: 'demo-large',
    title: '大',
    size: WidgetSize.LARGE,
    position: { col: 5, row: 1 },
    component: 'DemoLarge',
    props: { emoji: '📈', caption: '趋势统计' },
  },
  {
    id: 'demo-wide',
    title: '宽横幅',
    size: WidgetSize.WIDE,
    position: { col: 1, row: 3 },
    component: 'DemoWide',
    props: { emoji: '🎯', caption: '目标提醒横幅' },
  },
  {
    id: 'demo-tall',
    title: '高条',
    size: WidgetSize.TALL,
    position: { col: 4, row: 3 },
    component: 'DemoTall',
    props: { emoji: '📋', caption: '任务列表' },
  },
  {
    id: 'demo-xlarge',
    title: '超大',
    size: WidgetSize.XLARGE,
    position: { col: 5, row: 3 },
    component: 'DemoXlarge',
    props: { emoji: '🗂️', caption: '仪表盘' },
  },
  {
    id: 'demo-full',
    title: '全尺寸',
    size: WidgetSize.FULL,
    position: { col: 1, row: 6 },
    component: 'DemoFull',
    props: { emoji: '🖥️', caption: '全屏工作台' },
  },
]

/** 卡片内演示内容：按尺寸档位渲染不同形态 */
function sizeDesc(size: WidgetSize): string {
  const map = WIDGET_SIZE_MAP[size]
  return `${map.cols}×${map.rows}`
}

function emojiFor(id: string): string {
  const w = demoWidgets.find((d) => d.id === id)
  return (w?.props?.emoji as string | undefined) ?? '🧩'
}

function captionFor(id: string): string {
  const w = demoWidgets.find((d) => d.id === id)
  return (w?.props?.caption as string | undefined) ?? ''
}
</script>

<template>
  <div class="widget-desktop-tab">
    <WidgetDesktop
      :widgets="demoWidgets"
      :layout="layout"
      :columns="6"
      gap="12px"
      :stack-auto-size="true"
      :stack-max-visible="5"
      :stack-sensitivity="12"
      :stack-random-rotation="true"
      :stack-send-to-back-on-click="true"
      :stack-interaction-mode="'send-to-back'"
      :stack-autoplay="false"
      :stack-stiffness="180"
      :stack-damping="14"
      :stack-tilt-amount="8"
      :stack-drag-elastic="0.6"
      :stack-top-only-draggable="true"
      :stack-loop="true"
      @update:layout="onLayoutChange"
    >
      <!-- 网格卡片内容：按尺寸展示 -->
      <template #card="{ item }">
        <div class="demo-card">
          <div class="demo-card__head">
            <span class="demo-card__emoji">{{ emojiFor(item.id) }}</span>
            <span class="demo-card__title">{{ item.title }}</span>
            <span class="demo-card__size">{{ sizeDesc(item.size) }}</span>
          </div>
          <p class="demo-card__caption">{{ captionFor(item.id) }}</p>
          <div class="demo-card__body">
            <span
              v-for="n in Math.min(WIDGET_SIZE_MAP[item.size]?.cols ?? 2, 3)"
              :key="n"
              class="demo-card__dot"
            />
          </div>
        </div>
      </template>

      <!-- 堆叠卡片内容 -->
      <template #stack-card="{ item }">
        <div class="demo-card demo-card--stack">
          <span class="demo-card__emoji">{{ emojiFor(item.id) }}</span>
          <span class="demo-card__title">{{ item.title }}</span>
        </div>
      </template>
    </WidgetDesktop>
  </div>
</template>

<style scoped>
.widget-desktop-tab {
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

/* 演示卡片内容 */
.demo-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px;
  height: 100%;
  box-sizing: border-box;
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.08), rgba(74, 126, 255, 0.02));
  border: 1px solid rgba(74, 126, 255, 0.14);
  border-radius: 10px;
  overflow: hidden;
}

.demo-card--stack {
  align-items: center;
  justify-content: center;
  gap: 8px;
  text-align: center;
}

.demo-card__head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.demo-card__emoji {
  font-size: 16px;
  line-height: 1;
  flex-shrink: 0;
}

.demo-card__title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text, #e9ebf1);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.demo-card__size {
  margin-left: auto;
  font-size: 10px;
  font-family: monospace;
  color: var(--muted, #9ba2b2);
  background: rgba(74, 126, 255, 0.12);
  padding: 1px 5px;
  border-radius: 999px;
  flex-shrink: 0;
}

.demo-card__caption {
  margin: 0;
  font-size: 11px;
  color: var(--muted, #9ba2b2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.demo-card__body {
  display: flex;
  gap: 4px;
  margin-top: auto;
}

.demo-card__dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--primary, #4a7eff);
  opacity: 0.55;
}

.demo-card__dot:nth-child(2) {
  opacity: 0.8;
}

.demo-card__dot:nth-child(3) {
  opacity: 1;
}
</style>
