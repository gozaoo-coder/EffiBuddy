<script setup lang="ts">
/**
 * ProcessSection —— 推理过程 + 工具调用的合并展示区块
 *
 * 设计目标：让 agent 回复像文档一样简洁——
 * - 推理与工具调用合并为单行摘要标题（"已思考 8 秒 · 使用了 3 个工具"）
 * - 进行中（思考中 / 工具执行中）自动展开，全部完成后自动折叠
 * - 展开后：按流式到达顺序穿插展示思考文字与工具执行结果（segments），
 *   工具结果直接嵌在思考文字之间，而非与思考文字隔开单独成块；
 *   点击工具行可弹出完整参数与返回结果
 * - 点击标题行可随时手动展开/折叠
 */
import { ref, computed, watch, nextTick, onUnmounted } from 'vue'
import { animate } from 'animejs'
import { Icon } from '../basic'
import ToolCallGroup from '../ToolCallGroup.vue'
import type { ProcessSegment } from '../../types'

const props = withDefaults(
  defineProps<{
    /** 推理过程段（思考文字与工具调用按到达顺序穿插） */
    segments?: ProcessSegment[]
    /** 是否仍在思考中 */
    isThinking?: boolean
  }>(),
  {
    segments: () => [],
    isThinking: false,
  },
)

// 工具调用记录（从 segments 提取，供忙碌判定与标题统计）
const toolCalls = computed(() =>
  props.segments.filter((s) => s.kind === 'tool').map((s) => s.call),
)

// 是否忙碌：思考中或仍有工具在执行
const busy = computed(
  () => props.isThinking || toolCalls.value.some((c) => c.pending),
)

// 历史消息（加载时已全部完成）默认折叠；进行中默认展开
const collapsed = ref(!busy.value)
const bodyRef = ref<HTMLElement | null>(null)

// 已思考时长（秒）：多段推理累计；思考中实时刷新
const thinkStart = ref<number>(0)
const thinkDuration = ref<number>(0)
const liveElapsed = ref<number>(0)
let tickTimer: ReturnType<typeof setInterval> | null = null

watch(
  () => props.isThinking,
  (thinking, was) => {
    if (thinking && !was) {
      thinkStart.value = Date.now()
      liveElapsed.value = 0
      startTicker()
    } else if (!thinking && was) {
      stopTicker()
      if (thinkStart.value) {
        thinkDuration.value += Math.max(
          1,
          Math.round((Date.now() - thinkStart.value) / 1000),
        )
        thinkStart.value = 0
      }
    }
  },
)

// 思考中每秒刷新"思考中 X 秒"文案
function startTicker() {
  if (tickTimer) return
  tickTimer = setInterval(() => {
    if (props.isThinking && thinkStart.value) {
      liveElapsed.value = Math.max(
        1,
        Math.round((Date.now() - thinkStart.value) / 1000),
      )
    }
  }, 1000)
}

function stopTicker() {
  if (tickTimer) {
    clearInterval(tickTimer)
    tickTimer = null
  }
}

onUnmounted(stopTicker)

// 进行中 → 展开；全部完成 → 短暂延迟后自动折叠
let collapseTimer: ReturnType<typeof setTimeout> | null = null
watch(
  busy,
  (b, was) => {
    if (collapseTimer) {
      clearTimeout(collapseTimer)
      collapseTimer = null
    }
    if (b && !was) {
      // 进行中 → 直接展开（无动画）
      collapsed.value = false
    } else if (!b && was) {
      collapseTimer = setTimeout(() => {
        collapseNow()
      }, 600)
    }
  },
)

function toggle() {
  if (collapseTimer) {
    clearTimeout(collapseTimer)
    collapseTimer = null
  }
  if (collapsed.value) expandNow()
  else collapseNow()
}

// 展开：直接显示（无动画），并把思考内容定位到底部
function expandNow() {
  collapsed.value = false
  nextTick(() => {
    const el = bodyRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

// 合并：仅高度变小动画（不做透明度变化）
function collapseNow() {
  const el = bodyRef.value
  if (el) {
    animate(el, {
      maxHeight: [`${el.scrollHeight}px`, '0px'],
      duration: 200,
      ease: 'inOut(2)',
      onComplete: () => {
        collapsed.value = true
        el.style.maxHeight = ''
      },
    })
  } else {
    collapsed.value = true
  }
}

// 内容签名：思考文字 / 工具状态变化时滚动到底部（流式跟随）
const contentSig = computed(() =>
  props.segments
    .map((s) =>
      s.kind === 'reasoning'
        ? s.text
        : `${s.call.tool_name}:${s.call.result ?? ''}:${s.call.pending}`,
    )
    .join('\u0000'),
)
watch(contentSig, () => {
  if (collapsed.value) return
  nextTick(() => {
    const el = bodyRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
})

// 标题摘要文案：推理时长 + 工具数量合并
const titleText = computed(() => {
  const parts: string[] = []
  if (props.isThinking) {
    parts.push(
      liveElapsed.value > 0 ? `思考中 ${liveElapsed.value} 秒` : '思考中',
    )
  } else if (props.segments.some((s) => s.kind === 'reasoning' && s.text)) {
    parts.push(
      thinkDuration.value > 0 ? `已思考 ${thinkDuration.value} 秒` : '推理过程',
    )
  }
  if (toolCalls.value.length) {
    const pending = toolCalls.value.filter((c) => c.pending).length
    parts.push(
      pending > 0
        ? `执行工具中 ${toolCalls.value.length - pending}/${toolCalls.value.length}`
        : toolCalls.value.length === 1
          ? '使用了工具'
          : `使用了 ${toolCalls.value.length} 个工具`,
    )
  }
  return parts.join(' · ')
})
</script>

<template>
  <div class="process-section" :class="{ collapsed }">
    <!-- 合并标题行：点击切换折叠 -->
    <div class="process-header" @click="toggle">
      <span class="process-icon"><Icon name="thinking" :size="14" /></span>
      <span class="process-title">{{ titleText }}</span>
      <span v-if="busy" class="process-dots">
        <span class="dot"></span><span class="dot"></span><span class="dot"></span>
      </span>
      <span class="process-arrow">
        <Icon :name="collapsed ? 'chevron-right' : 'chevron-down'" :size="12" />
      </span>
    </div>

    <!-- 展开内容：思考文字与工具执行结果按到达顺序穿插展示 -->
    <div v-show="!collapsed" ref="bodyRef" class="process-body">
      <template v-for="(seg, i) in segments" :key="i">
        <div v-if="seg.kind === 'reasoning'" class="process-reasoning">{{ seg.text }}</div>
        <ToolCallGroup v-else :calls="[seg.call]" embedded show-result />
      </template>
    </div>
  </div>
</template>

<style scoped>
/* 无卡片外观：以文档流形式融入 assistant 回复；
   左侧缩进与正文形成明确的层级区分 */
.process-section {
  padding-left: 2px;
  font-size: 13px;
  color: var(--muted, #888);
}

.process-header {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 8px;
  margin-left: -8px;
  border-radius: var(--radius-md);
  cursor: pointer;
  user-select: none;
  transition: background var(--duration-fast, 120ms) var(--ease-standard, ease);
}

.process-header:hover {
  background: var(--card-2, rgba(0, 0, 0, 0.04));
}

.process-icon {
  line-height: 1;
  color: var(--muted, #888);
}

.process-title {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--muted, #888);
  white-space: nowrap;
}

/* 进行中跳动的点 */
.process-dots {
  display: inline-flex;
  gap: 3px;
}

.process-dots .dot {
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: var(--muted, #aaa);
  animation: process-bounce 1.2s infinite ease-in-out;
}

.process-dots .dot:nth-child(2) {
  animation-delay: 0.15s;
}

.process-dots .dot:nth-child(3) {
  animation-delay: 0.3s;
}

@keyframes process-bounce {
  0%, 80%, 100% {
    transform: translateY(0);
    opacity: 0.5;
  }
  40% {
    transform: translateY(-3px);
    opacity: 1;
  }
}

.process-arrow {
  line-height: 1;
  color: var(--muted, #888);
}

/* 展开内容：左侧细线标识层级，缩进与上下 margin 加大，与标题行/正文明确区分；
   整块可滚动（思考 + 工具穿插后统一滚动），overflow-y:auto 供流式跟随；
   overflow:hidden 由折叠动画时临时覆盖（collapseNow 内联 maxHeight） */
.process-body {
  margin: 8px 0 6px 8px;
  padding: 2px 0 2px 12px;
  border-left: 2px solid var(--border, rgba(0, 0, 0, 0.08));
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 320px;
  overflow-y: auto;
  overflow-x: hidden;
}

/* 思考文字段：文档流样式，不单独滚动，跟随整体滚动 */
.process-reasoning {
  padding: 2px 6px 2px 0;
  font-size: 12.5px;
  line-height: 1.6;
  color: var(--muted, #777);
  white-space: pre-wrap;
  word-break: break-word;
}

.process-body::-webkit-scrollbar {
  width: 6px;
}

.process-body::-webkit-scrollbar-thumb {
  background: var(--border, rgba(0, 0, 0, 0.152));
  border-radius: 3px;
}
</style>
