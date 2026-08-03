<script setup lang="ts">
/**
 * TaskBubble —— 长程任务气泡（单组）
 *
 * 每个 TaskGroup 渲染为一个 TaskBubble：
 * - 头部：任务标题 + 进度摘要 + 折叠箭头
 * - todoTree：仅活跃组展示当前 todoTree；历史组展示「已完成」标记
 * - 实时工作输出容器：本组全部 assistant 消息（不截断）
 *
 * 动画管线（全流程）：
 * - 气泡进入：scale + opacity 过渡（由父级 renderList 的 key 触发）
 * - 折叠/展开：max-height + opacity 过渡，anime.js 驱动
 *   · 中断处理：anime.js 自动覆盖正在进行的动画，无缝接续
 * - 输出展开/收起：max-height 在 440px ↔ none 间切换
 */
import { computed, inject, nextTick, watch, onUnmounted } from 'vue'
import { animate } from 'animejs'
import { Icon } from '../basic'
import TodoTreeNode from '../todo/TodoTreeNode.vue'
import AssistantContent from './AssistantContent.vue'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import { useAutoScroll } from '../../composables/chat/useAutoScroll'
import type { TaskGroup } from '../../composables/chat/useTaskMode'

const props = defineProps<{
  /** 本气泡对应的任务组 */
  group: TaskGroup
}>()

const store = inject(CHAT_STORE_KEY)!
const {
  todoTree,
  todoItems,
  taskSummary,
  groupMessages,
  isGroupCollapsed,
  toggleGroupCollapsed,
  isGroupOutputExpanded,
  toggleGroupOutputExpanded,
} = store.taskMode
const { getMeta, streamingBubbleId } = store.streaming
const { isDark } = store.core

// ── 会话版本操作（与普通消息气泡 msg-hover-bar 同款）：作用于本组起始消息 ──
const { versioning } = store
const { onCopy, onBranch, onSaveTemp, onRollback, onUndoBefore } = versioning
const startMsg = computed(
  () => store.core.messages.value.find((m) => m.id === props.group.startMessageId) ?? null,
)
/** 活跃组正在流式输出时不展示版本操作栏（避免对运行中的任务组做破坏性操作） */
const isGroupStreaming = computed(() =>
  props.group.active && groupMessages(props.group).some((m) => m.id === streamingBubbleId.value),
)

// ── 实时工作输出自动滚动 ────────────────────────────────────────────────
// 复用 chat 列表的 useAutoScroll：流式 token 增长时跟随底部，
// 用户上滑阅读时暂停跟随，滑回底部恢复。每个 TaskBubble 实例独立一份。
const { scroller, stickToBottom, scrollBottom, attachScroller, dispose } = useAutoScroll()

watch(scroller, (el, oldEl) => {
  attachScroller(el, oldEl ?? null)
})

onUnmounted(() => {
  dispose()
})

// ── 折叠动画 ────────────────────────────────────────────────────────────
let animating = false

function toggle() {
  if (animating) return
  animating = true
  const g = props.group
  const wasCollapsed = isGroupCollapsed(g)

  if (!wasCollapsed) {
    // 折叠：先把高度动画到 0
    const el = scroller.value
    if (!el) {
      toggleGroupCollapsed(g)
      animating = false
      return
    }
    animate(el, {
      maxHeight: [`${el.scrollHeight}px`, '0px'],
      opacity: [1, 0],
      duration: 200,
      ease: 'inOut(2)',
      onComplete: () => {
        toggleGroupCollapsed(g)
        el.style.maxHeight = ''
        animating = false
      },
    })
  } else {
    // 展开：先取消折叠，下一帧测量高度并动画
    toggleGroupCollapsed(g)
    // 展开后重置跟随，确保动画结束展示最新输出
    stickToBottom.value = true
    nextTick(() => {
      const el = scroller.value
      if (!el) {
        animating = false
        return
      }
      const h = el.scrollHeight
      animate(el, {
        maxHeight: ['0px', `${h}px`],
        opacity: [0, 1],
        duration: 260,
        ease: 'out(3)',
        onComplete: () => {
          el.style.maxHeight = ''
          // 动画结束后滚到底部，展示最新输出
          stickToBottom.value = true
          scrollBottom()
          animating = false
        },
      })
    })
  }
}
</script>

<template>
  <div
    class="task-bubble"
    :class="{ 'task-bubble--active': group.active, 'task-bubble--history': !group.active }"
  >
      <!-- 会话版本操作 hover 操作栏（与消息气泡同款，作用于本组起始消息） -->
      <div
        v-if="startMsg && !isGroupStreaming"
        class="task-version-bar"
        @pointerdown.stop
        @click.stop
      >
        <button
          type="button"
          class="task-version-btn"
          title="复制信息"
          @click.stop="onCopy(startMsg)"
        >
          <Icon name="copy" :size="14" />
          <span class="task-version-tip">复制信息</span>
        </button>
        <button
          type="button"
          class="task-version-btn"
          title="开启分支"
          @click.stop="onBranch(startMsg)"
        >
          <Icon name="branch" :size="14" />
          <span class="task-version-tip">开启分支：从此消息另起一条对话线</span>
        </button>
        <button
          type="button"
          class="task-version-btn"
          title="保存临时版本"
          @click.stop="onSaveTemp(startMsg)"
        >
          <Icon name="bookmark" :size="14" />
          <span class="task-version-tip">保存临时版本：在此消息打版本书签</span>
        </button>
        <button
          type="button"
          class="task-version-btn"
          title="回溯版本"
          @click.stop="onRollback(startMsg)"
        >
          <Icon name="refresh" :size="14" />
          <span class="task-version-tip">回溯版本：对话重置到此消息（其后移除）</span>
        </button>
        <button
          type="button"
          class="task-version-btn"
          title="撤回至此消息前"
          @click.stop="onUndoBefore(startMsg)"
        >
          <Icon name="undo" :size="14" />
          <span class="task-version-tip">撤回至此消息前：删除此消息及其后全部</span>
        </button>
      </div>

    <div class="task-bubble-head" @click="toggle">
      <Icon :name="group.active ? 'wrench' : 'check-circle'" :size="16" />
      <span class="task-bubble-title">
        {{ group.active ? '长程任务' : '历史任务' }}
      </span>
      <span v-if="group.active" class="task-bubble-summary">{{ taskSummary }}</span>
      <span v-else class="task-bubble-summary task-bubble-summary--done">已完成</span>
      <Icon
        :name="isGroupCollapsed(group) ? 'chevron-down' : 'chevron-up'"
        :size="15"
        class="task-bubble-chevron"
      />
    </div>

    <!-- todoTree 实现状态（仅活跃组展示当前 todoTree） -->
    <div v-if="group.active && !isGroupCollapsed(group)" class="task-tree">
      <template v-if="todoTree.length">
        <TodoTreeNode
          v-for="node in todoTree"
          :key="node.id"
          :node="node"
          :depth="0"
          :items="todoItems"
          :editing="{}"
          :gen-id="() => ''"
          :on-changed="() => {}"
          readonly
        />
      </template>
      <div v-else class="task-tree-empty">正在建立任务清单…</div>
    </div>

    <!-- 实时工作输出容器：本组全部 assistant 输出 -->
    <div
      v-show="!isGroupCollapsed(group)"
      ref="scroller"
      class="task-container"
      :class="{ 'task-container--expanded': isGroupOutputExpanded(group) }"
    >
      <div class="task-container-bar">
        <span class="task-container-label">
          {{ group.active ? '实时工作输出' : '工作输出' }}
        </span>
        <button
          type="button"
          class="task-container-toggle"
          :title="isGroupOutputExpanded(group) ? '收起输出' : '展开全部输出'"
          @click.stop="toggleGroupOutputExpanded(group)"
        >
          <Icon :name="isGroupOutputExpanded(group) ? 'chevron-down' : 'chevron-up'" :size="13" />
          <span>{{ isGroupOutputExpanded(group) ? '收起' : '展开' }}</span>
        </button>
      </div>
      <template v-if="groupMessages(group).length">
        <div v-for="m in groupMessages(group)" :key="m.id" class="task-msg">
          <AssistantContent
            :message="m"
            :meta="getMeta(m.id)"
            :is-streaming="m.id === streamingBubbleId"
            :is-dark="isDark"
          />
        </div>
      </template>
      <div v-else class="task-container-empty">等待 agent 开始工作…</div>
    </div>
  </div>
</template>

<style scoped>
/* ============================================================
 * 长程任务气泡 —— 严格遵循 design tokens（4px 基线 / fs / radius / duration / ease）
 * 视觉与同列表 msg-bubble 对齐：card 背景 + border + shadow-sm
 * ============================================================ */

.task-bubble {
  position: relative; /* hover 版本操作栏的定位基准 */
  max-width: 100%;
  min-width: 0; /* flex 子项允许收缩，防内容撑破父容器 */
  align-self: stretch;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-xl);
  overflow: hidden;
  min-height: fit-content;
  box-shadow: var(--shadow-sm);
  transition: box-shadow var(--duration-base) var(--ease-standard),
    border-color var(--duration-base) var(--ease-standard);
}

.task-bubble--active:hover {
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  box-shadow: var(--shadow);
}

.task-bubble--history {
  opacity: 0.92;
}

.task-bubble--history:hover {
  opacity: 1;
  border-color: color-mix(in srgb, var(--success) 25%, var(--border));
  box-shadow: var(--shadow);
}

/* 会话版本操作 hover 操作栏（与消息气泡 msg-hover-bar 同款） */
.task-version-bar {
  position: absolute;
  top: 6px;
  right: 8px;
  z-index: 20;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 3px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-md);
  opacity: 0;
  pointer-events: none;
  transform: translateY(-3px);
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.task-bubble:hover .task-version-bar {
  opacity: 1;
  pointer-events: auto;
  transform: translateY(0);
}

.task-version-btn {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: color 0.12s ease, background 0.12s ease;
}

.task-version-btn:hover {
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 12%, transparent);
}

/* 按钮 tooltip：悬浮时在下方弹出文字提示 */
.task-version-tip {
  position: absolute;
  top: calc(100% + 6px);
  left: 50%;
  transform: translateX(-50%);
  z-index: 30;
  width: max-content;
  max-width: 220px;
  padding: 4px 9px;
  font-size: 12px;
  line-height: 1.4;
  color: var(--text);
  text-align: center;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-md);
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.12s ease;
  white-space: normal;
}

.task-version-btn:hover .task-version-tip {
  opacity: 1;
}


/* 头部：标题 + 进度摘要 + 折叠箭头 */
.task-bubble-head {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  cursor: pointer;
  user-select: none;
  border-bottom: 1px solid var(--border);
  transition: background var(--duration-fast) var(--ease-standard);
}

.task-bubble--active .task-bubble-head {
  background: linear-gradient(
    135deg,
    color-mix(in srgb, var(--primary) 12%, var(--card)),
    var(--card) 55%
  );
}

.task-bubble--active .task-bubble-head:hover {
  background: linear-gradient(
    135deg,
    color-mix(in srgb, var(--primary) 16%, var(--card)),
    var(--card) 60%
  );
}

.task-bubble--history .task-bubble-head {
  background: linear-gradient(
    135deg,
    color-mix(in srgb, var(--success) 8%, var(--card)),
    var(--card) 55%
  );
}

.task-bubble-title {
  font-weight: 700;
  font-size: var(--fs-md);
  color: var(--text);
}

.task-bubble-summary {
  font-size: var(--fs-sm);
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  margin-left: auto;
  white-space: nowrap;
}

.task-bubble-summary--done {
  color: var(--success);
}

.task-bubble-chevron {
  color: var(--muted);
  flex-shrink: 0;
  transition: transform var(--duration-base) var(--ease-emphasized);
}

/* 任务清单 todoTree 区：紧凑模式 */
.task-tree {
  padding: var(--space-1) var(--space-2);
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  border-bottom: 1px dashed var(--border);
  background: color-mix(in srgb, var(--primary) 3%, var(--card));
}

.task-tree-empty {
  font-size: var(--fs-xs);
  color: var(--muted);
  padding: 2px 0;
}

/* 只读 TodoTreeNode 在气泡内的尺寸增强（穿透组件 scoped 样式） */
.task-bubble :deep(.ttn) {
  gap: 1px;
}

.task-bubble :deep(.ttn-row) {
  padding: 2px var(--space-1);
  gap: var(--space-1);
  border-radius: var(--radius-xs);
}

.task-bubble :deep(.ttn-status) {
  width: var(--fs-md);
  height: var(--fs-md);
  color: var(--muted);
}

.task-bubble :deep(.ttn-status.completed) {
  color: var(--success);
}

.task-bubble :deep(.ttn-status.in_progress) {
  color: var(--primary);
}

.task-bubble :deep(.ttn-content) {
  font-size: var(--fs-sm);
  color: var(--text);
  white-space: normal;
  word-break: break-word;
  line-height: 1.45;
}

.task-bubble :deep(.ttn-content.done) {
  color: var(--muted);
  text-decoration: line-through;
}

/* 实时工作输出容器：滚动区背景 bg-2，与 msg-list 滚动容器一致 */
.task-container {
  padding: 0 var(--space-4) var(--space-4);
  max-height: 440px;
  overflow-y: auto;
  overflow-x: hidden; /* 任务卡片内容不允许横向溢出，纵向滚动即可 */
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  background: var(--bg-2);
}

/* 滚动条与 msg-list 同款 */
.task-container::-webkit-scrollbar {
  width: 8px;
}
.task-container::-webkit-scrollbar-thumb {
  background: var(--border);
  border-radius: var(--radius-xs);
}
.task-container::-webkit-scrollbar-thumb:hover {
  background: var(--muted);
}

/* 展开态：突破高度上限 */
.task-container--expanded {
  max-height: none;
}

/* 顶部 sticky 工具条：内边距等价于容器原 padding-top，
   确保背景完全覆盖滚动区顶部，消除透明缝 */
.task-container-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  padding: var(--space-3) 0 var(--space-2);
  border-bottom: 1px solid var(--border);
  position: sticky;
  top: 0;
  background: var(--bg-2);
  z-index: 1;
}

.task-container-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

/* 收起/展开按钮：chips 风格，复用 tokens */
.task-container-toggle {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-2);
  font-size: var(--fs-sm);
  color: var(--muted);
  background: transparent;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: color var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard);
}

.task-container-toggle:hover {
  color: var(--primary);
  border-color: var(--primary);
}

/* 单条 assistant 输出 */
.task-msg {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  /* 实时工作输出正文缩放至 75%：
     子组件（MarkdownRender/ReasoningBox/ToolCallGroup/SubAgentCard）
     内部均使用固定像素字号，font-size 继承无法穿透，
     故用 zoom 整体缩放渲染（保留布局流，不残留原占位）。 */
  zoom: 0.75;
}

.task-msg + .task-msg {
  padding-top: var(--space-2);
  border-top: 1px dashed var(--border);
}

.task-container-empty {
  font-size: var(--fs-sm);
  color: var(--muted);
  padding: var(--space-2) 0;
}
</style>
