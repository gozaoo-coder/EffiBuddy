<script setup lang="ts">
/**
 * WidgetItem —— 可交互小组件内容示例
 *
 * 展示如何在一个 WidgetCard 中嵌入自定义交互内容。
 * 可作为其他 widget 组件的模板。
 */
import { ref } from 'vue'

const props = withDefaults(
  defineProps<{
    /** 标题 */
    title?: string
    /** 初始计数 */
    initialCount?: number
    /** 显示刷新按钮 */
    showRefresh?: boolean
    /** 显示进度条 */
    showProgress?: boolean
    /** 列表项 */
    items?: string[]
  }>(),
  {
    title: '小组件',
    initialCount: 0,
    showRefresh: true,
    showProgress: false,
    items: () => [],
  },
)

const emit = defineEmits<{
  (e: 'action', action: string): void
  (e: 'refresh'): void
}>()

const count = ref(props.initialCount)
const progress = ref(0)
let progressTimer: ReturnType<typeof setInterval> | null = null

function increment() {
  count.value++
  emit('action', 'increment')
}

function decrement() {
  count.value--
  emit('action', 'decrement')
}

function handleRefresh() {
  emit('refresh')
}

function toggleProgress() {
  if (progressTimer) {
    clearInterval(progressTimer)
    progressTimer = null
    return
  }
  progress.value = 0
  progressTimer = setInterval(() => {
    progress.value += 2
    if (progress.value >= 100) {
      progress.value = 100
      if (progressTimer) {
        clearInterval(progressTimer)
        progressTimer = null
      }
    }
  }, 50)
}
</script>

<template>
  <div class="widget-item">
    <!-- 计数交互 -->
    <div class="widget-item__counter">
      <button class="widget-item__btn" @click="decrement">−</button>
      <span class="widget-item__count">{{ count }}</span>
      <button class="widget-item__btn" @click="increment">+</button>
    </div>

    <!-- 进度条 -->
    <div v-if="showProgress" class="widget-item__progress">
      <div class="widget-item__progress-track">
        <div
          class="widget-item__progress-bar"
          :style="{ width: `${progress}%` }"
        />
      </div>
      <button class="widget-item__btn widget-item__btn--sm" @click="toggleProgress">
        {{ progressTimer ? '暂停' : '开始' }}
      </button>
    </div>

    <!-- 列表项 -->
    <ul v-if="items.length > 0" class="widget-item__list">
      <li v-for="(item, i) in items" :key="i" class="widget-item__list-item">
        {{ item }}
      </li>
    </ul>

    <!-- 刷新按钮 -->
    <div v-if="showRefresh" class="widget-item__footer">
      <button class="widget-item__btn widget-item__btn--primary" @click="handleRefresh">
        刷新
      </button>
    </div>
  </div>
</template>

<style scoped>
.widget-item {
  display: flex;
  flex-direction: column;
  gap: 10px;
  height: 100%;
}

/* 计数器 */
.widget-item__counter {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
}

.widget-item__count {
  font-size: 24px;
  font-weight: 600;
  color: var(--text, #e9ebf1);
  min-width: 40px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}

/* 通用按钮 */
.widget-item__btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: 1px solid var(--border, rgba(255, 255, 255, 0.09));
  border-radius: var(--radius-sm, 3px);
  background: var(--card-2, rgb(46, 46, 51));
  color: var(--text, #e9ebf1);
  cursor: pointer;
  font-size: 16px;
  font-weight: 500;
  transition: background 0.15s, border-color 0.15s;
}

.widget-item__btn:hover {
  background: var(--hover, rgba(255, 255, 255, 0.1));
  border-color: var(--border-strong, rgba(255, 255, 255, 0.16));
}

.widget-item__btn--sm {
  width: 28px;
  height: 28px;
  font-size: 12px;
}

.widget-item__btn--primary {
  width: auto;
  padding: 0 16px;
  font-size: 12px;
  background: var(--primary, #5b8cff);
  border-color: transparent;
  color: #fff;
}

.widget-item__btn--primary:hover {
  opacity: 0.9;
}

/* 进度条 */
.widget-item__progress {
  display: flex;
  align-items: center;
  gap: 8px;
}

.widget-item__progress-track {
  flex: 1;
  height: 6px;
  background: var(--ring-track, rgba(255, 255, 255, 0.15));
  border-radius: 3px;
  overflow: hidden;
}

.widget-item__progress-bar {
  height: 100%;
  background: var(--primary, #5b8cff);
  border-radius: 3px;
  transition: width 0.1s linear;
}

/* 列表 */
.widget-item__list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  overflow: auto;
}

.widget-item__list-item {
  padding: 4px 8px;
  font-size: 11px;
  color: var(--muted, #9ba2b2);
  background: var(--bg-2, rgb(32, 32, 36));
  border-radius: var(--radius-sm, 3px);
}

/* 底部 */
.widget-item__footer {
  display: flex;
  justify-content: center;
  padding-top: 4px;
}
</style>