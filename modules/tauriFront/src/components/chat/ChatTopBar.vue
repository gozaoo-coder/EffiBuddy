<script setup lang="ts">
/**
 * ChatTopBar —— 聊天主区顶部悬浮顶栏
 *
 * - 左侧：标题面包屑
 *   · 默认态：`[ 标题 ]`，点击进入内联编辑以修改会话标题（editable=true）
 *   · 子代理态：`[ 父标题 ] / [ 子代理标题 ]`，点击父标题返回父会话（subTitle 存在时 title 视为父标题）
 * - 右侧：收起上下文面板按钮 + 上下文用量 ring（hover 浮出文字说明）
 *
 * 定位：float=true 时 position:absolute + 透明背景悬浮于消息列表之上；
 * float=false 时按正常流渲染（子代理视图/inline 复用）。
 */
import { ref, computed, nextTick } from 'vue'
import { Icon } from '../basic'
import ContextRing from '../basic/ContextRing.vue'

const props = withDefaults(
  defineProps<{
    title: string
    /** 子代理标题：存在时切换为面包屑模式 */
    subTitle?: string | null
    /** 是否以 absolute 悬浮（主聊天 true，子代理视图 false） */
    float?: boolean
    /** 是否显示上下文 ring */
    showRing?: boolean
    used?: number
    max?: number
    /** 是否显示“收起面板”按钮 */
    showPanel?: boolean
    panelOpen?: boolean
    /** 是否允许内联编辑标题（默认态 true） */
    editable?: boolean
  }>(),
  {
    subTitle: null,
    float: true,
    showRing: false,
    used: 0,
    max: 0,
    showPanel: false,
    panelOpen: false,
    editable: true,
  },
)

const emit = defineEmits<{
  (e: 'edit-title', title: string): void
  (e: 'back-to-parent'): void
  (e: 'toggle-panel'): void
}>()

// ---- 上下文 ring hover 文案 ----
const ringText = computed(() => {
  if (!props.max) return ''
  const pct = Math.min(100, Math.round((props.used / props.max) * 100))
  return `上下文用量 ${props.used.toLocaleString()} / ${props.max.toLocaleString()} tokens · ${pct}%`
})

// ---- 内联编辑标题 ----
const editing = ref(false)
const draft = ref('')
const editRef = ref<HTMLInputElement | null>(null)

function onTitleClick() {
  if (props.subTitle) {
    emit('back-to-parent')
    return
  }
  if (props.editable) startEdit()
}

function startEdit() {
  editing.value = true
  draft.value = props.title
  nextTick(() => editRef.value?.focus())
}

function commitEdit() {
  editing.value = false
  const t = draft.value.trim()
  if (t && t !== props.title) emit('edit-title', t)
}

function cancelEdit() {
  editing.value = false
}
</script>

<template>
  <div class="chat-topbar" :class="{ 'chat-topbar--inline': !float }">
    <!-- 左侧：标题 / 面包屑 -->
    <div class="topbar-left">
      <input
        v-if="editing"
        ref="editRef"
        v-model="draft"
        class="topbar-edit"
        :placeholder="title"
        @keydown.enter="commitEdit"
        @keydown.esc="cancelEdit"
        @blur="commitEdit"
        @click.stop
      />
      <template v-else>
        <template v-if="subTitle">
          <button
            type="button"
            class="topbar-crumb topbar-crumb--parent"
            :title="`回到 ${title}`"
            @click="emit('back-to-parent')"
          >
            <Icon name="arrow-left" :size="13" />
            <span class="topbar-crumb-text">{{ title || '父会话' }}</span>
          </button>
          <span class="topbar-sep">/</span>
          <span class="topbar-crumb topbar-crumb--current">
            <Icon name="robot" :size="13" />
            <span class="topbar-crumb-text">{{ subTitle }}</span>
          </span>
        </template>
        <button
          v-else
          type="button"
          class="topbar-crumb"
          :class="{ 'topbar-crumb--edit': editable }"
          :title="editable ? '点击修改标题' : ''"
          @click="onTitleClick"
        >
          <Icon :name="editable ? 'pencil' : 'chat'" :size="13" />
          <span class="topbar-crumb-text">{{ title || '新对话' }}</span>
        </button>
      </template>
    </div>

    <!-- 右侧：收起面板 + 上下文 ring（hover 弹出气泡提示） -->
    <div class="topbar-right">
      <button
        v-if="showPanel"
        type="button"
        class="topbar-btn"
        :class="{ 'topbar-btn--on': panelOpen }"
        :data-tooltip="panelOpen ? '收起上下文面板' : '展开上下文面板'"
        @click="emit('toggle-panel')"
      >
        <Icon name="discover" :size="14" />
      </button>

      <div v-if="showRing && max > 0" class="topbar-ring" :data-tooltip="ringText">
        <ContextRing :used="used" :max="max" :size="18" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-topbar {
  position: absolute;
  top: 10px;
  left: 10px;
  right: 10px;
  z-index: 30;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 4px 8px;
  background: transparent;
  pointer-events: none; /* 不拦截下方滚动，按钮自身重新启用 pointer-events */
}

.chat-topbar--inline {
  position: static;
  padding: 8px 16px;
  pointer-events: auto;
}

.topbar-left,
.topbar-right {
  display: flex;
  align-items: center;
  gap: 6px;
  pointer-events: auto;
  min-width: 0;
}

/* ---- 统一顶栏子元素基线：等高 28px / 圆角 7px / 字号 12px ---- */
.topbar-crumb,
.topbar-btn,
.topbar-ring,
.topbar-edit {
  height: 28px;
  padding: 0 8px;
  border-radius: 7px;
  font-size: 12px;
}

/* ---- 标题 / 面包屑 ---- */
.topbar-crumb {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 340px;
  font-weight: 600;
  color: var(--text);
  background: color-mix(in srgb, var(--bg) 78%, transparent);
  border: 1px solid var(--border);
  cursor: default;
  user-select: none;
}

.topbar-crumb--edit {
  cursor: pointer;
}

.topbar-crumb--edit:hover {
  border-color: var(--primary);
  color: var(--primary);
}

.topbar-crumb--parent {
  cursor: pointer;
  background: color-mix(in srgb, var(--primary) 8%, var(--bg));
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
}

.topbar-crumb--parent:hover {
  border-color: var(--primary);
  color: var(--primary);
}

.topbar-crumb--current {
  color: var(--muted);
  border-color: transparent;
  background: transparent;
}

.topbar-crumb-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.topbar-sep {
  color: var(--muted);
  font-size: 12px;
}

/* ---- 内联编辑输入 ---- */
.topbar-edit {
  width: 300px;
  max-width: 60vw;
  font-weight: 600;
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--primary);
  outline: none;
}

/* ---- 右侧按钮 / ring ---- */
.topbar-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  color: var(--muted);
  background: color-mix(in srgb, var(--bg) 78%, transparent);
  border: 1px solid transparent;
  cursor: pointer;
  transition: color 0.15s, border-color 0.15s, background 0.15s;
}

.topbar-btn:hover {
  color: var(--primary);
  border-color: var(--border);
  background: color-mix(in srgb, var(--primary) 8%, var(--bg));
}

.topbar-btn--on {
  color: var(--primary);
}

/* 上下文用量 ring：信息展示而非按钮——保持低调，hover 仅加深背景以暗示 tooltip */
.topbar-ring {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  color: var(--muted);
  background: color-mix(in srgb, var(--bg) 78%, transparent);
  border: 1px solid transparent;
  cursor: default;
  transition: background 0.15s;
}

.topbar-ring:hover {
  background: color-mix(in srgb, var(--bg) 60%, transparent);
}

/* ---- tippy 风格悬浮提示：hover 弹出气泡（::after + attr(data-tooltip)） ---- */
.topbar-btn,
.topbar-ring {
  position: relative;
}

.topbar-btn::after,
.topbar-ring::after {
  content: attr(data-tooltip);
  position: absolute;
  top: calc(100% + 8px);
  left: 50%;
  transform: translateX(-50%) translateY(-4px);
  max-width: 240px;
  padding: 4px 9px;
  font-size: 11px;
  line-height: 1.45;
  text-align: center;
  white-space: normal;
  color: var(--text);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: 7px;
  box-shadow: var(--shadow);
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.16s ease, transform 0.16s ease;
  z-index: 60;
}

.topbar-btn:hover::after,
.topbar-ring:hover::after {
  opacity: 1;
  transform: translateX(-50%) translateY(0);
}

/* 右侧控件：tooltip 右对齐锚定，避免长文案（如上下文用量）超出视口右缘被截断 */
.topbar-right .topbar-btn::after,
.topbar-right .topbar-ring::after {
  left: auto;
  right: 0;
  transform: translateY(-4px);
  text-align: left;
}

.topbar-right .topbar-btn:hover::after,
.topbar-right .topbar-ring:hover::after {
  transform: translateY(0);
}
</style>