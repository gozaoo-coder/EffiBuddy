<script setup lang="ts">
/**
 * ReasoningBox 推理折叠框
 *
 * 用于展示 LLM 的推理/思考链（DeepSeek-R1 / o1 等模型产生）。
 *
 * 设计要点：
 * - 标题区与折叠状态整体最高 38px
 * - 展开后内容区最高 200px，超出滚动
 * - 思考完成（isThinking: true → false）时自动折叠
 * - 标题栏可点击切换展开/折叠
 * - 展开/折叠使用 anime.js v4 动画（max-height + opacity）
 */
import { ref, watch, computed } from 'vue'
import { animate } from 'animejs'
import { Icon } from './basic'

const props = withDefaults(
  defineProps<{
    /** 推理文本 */
    content: string
    /** 是否仍在思考中 */
    isThinking?: boolean
  }>(),
  {
    isThinking: false,
  },
)

// 折叠状态：默认展开（思考中可见）
const collapsed = ref(false)
const bodyRef = ref<HTMLElement | null>(null)

// 已思考时长（秒）：用 isThinking 切换时间戳近似
const thinkStart = ref<number>(Date.now())
const thinkDuration = ref<number>(0)

watch(
  () => props.isThinking,
  (thinking, was) => {
    if (thinking && !was) {
      // 开始思考
      thinkStart.value = Date.now()
      collapsed.value = false
    } else if (!thinking && was) {
      // 思考结束：记录时长，自动折叠
      thinkDuration.value = Math.max(1, Math.round((Date.now() - thinkStart.value) / 1000))
      collapseNow()
    }
  },
)

// 切换折叠
function toggle() {
  if (collapsed.value) expandNow()
  else collapseNow()
}

function expandNow() {
  collapsed.value = false
  // 等待 DOM 更新后动画
  requestAnimationFrame(() => {
    const el = bodyRef.value
    if (!el) return
    animate(el, {
      maxHeight: ['0px', '200px'],
      opacity: [0, 1],
      duration: 280,
      ease: 'out(3)',
    })
  })
}

function collapseNow() {
  const el = bodyRef.value
  if (el) {
    animate(el, {
      maxHeight: ['200px', '0px'],
      opacity: [1, 0],
      duration: 220,
      ease: 'inOut(2)',
      onComplete: () => {
        collapsed.value = true
      },
    })
  } else {
    collapsed.value = true
  }
}

// 标题文案
const titleText = computed(() => {
  if (props.isThinking) return '思考中…'
  if (thinkDuration.value > 0) return `已思考 ${thinkDuration.value} 秒`
  return '推理过程'
})
</script>

<template>
  <div class="reasoning-box" :class="{ collapsed }">
    <!-- 标题栏：高度 54px，点击切换 -->
    <div class="reasoning-header" @click="toggle">
      <span class="reasoning-icon"><Icon name="thinking" :size="16" /></span>
      <span class="reasoning-title">{{ titleText }}</span>
      <span v-if="isThinking" class="reasoning-dots">
        <span class="dot"></span><span class="dot"></span><span class="dot"></span>
      </span>
      <span class="reasoning-arrow"><Icon :name="collapsed ? 'chevron-right' : 'chevron-down'" :size="12" /></span>
    </div>
    <!-- 内容区：展开时最高 200px，可滚动 -->
    <div v-show="!collapsed" ref="bodyRef" class="reasoning-body">
      <div class="reasoning-text">{{ content }}</div>
    </div>
  </div>
</template>

<style scoped>
.reasoning-box {
  margin: 6px 0 8px;
  border-radius: var(--radius-md, 12px);
  background: var(--card-2, rgba(0, 0, 0, 0.04));
  border: 1px solid var(--border, rgba(0, 0, 0, 0.08));
  overflow: hidden;
  font-size: 13px;
  color: var(--muted, #888);
}

/* 标题栏：最高 38px */
.reasoning-header {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 38px;
  max-height: 38px;
  padding: 0 12px;
  cursor: pointer;
  user-select: none;
  transition: background var(--duration-fast, 120ms) var(--ease-standard, ease);
}

.reasoning-header:hover {
  background: var(--card, rgba(0, 0, 0, 0.06));
}

.reasoning-icon {
  font-size: 16px;
  line-height: 1;
}

.reasoning-title {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: 500;
  color: var(--text, #333);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 思考中跳动的点 */
.reasoning-dots {
  display: inline-flex;
  gap: 3px;
  margin-right: 8px;
}

.reasoning-dots .dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--muted, #aaa);
  animation: reasoning-bounce 1.2s infinite ease-in-out;
}

.reasoning-dots .dot:nth-child(2) {
  animation-delay: 0.15s;
}

.reasoning-dots .dot:nth-child(3) {
  animation-delay: 0.3s;
}

@keyframes reasoning-bounce {
  0%, 80%, 100% {
    transform: translateY(0);
    opacity: 0.5;
  }
  40% {
    transform: translateY(-3px);
    opacity: 1;
  }
}

.reasoning-arrow {
  font-size: 12px;
  color: var(--muted, #888);
  line-height: 1;
}

/* 内容区：最高 200px，超出滚动 */
.reasoning-body {
  max-height: 200px;
  overflow-y: auto;
  padding: 0 14px 12px;
  border-top: 1px solid var(--border, rgba(0, 0, 0, 0.05));
}

.reasoning-text {
  padding-top: 10px;
  font-size: 13px;
  line-height: 1.6;
  color: var(--muted, #777);
  white-space: pre-wrap;
  word-break: break-word;
}

.reasoning-body::-webkit-scrollbar {
  width: 6px;
}

.reasoning-body::-webkit-scrollbar-thumb {
  background: var(--border, rgba(0, 0, 0, 0.15));
  border-radius: 3px;
}
</style>
