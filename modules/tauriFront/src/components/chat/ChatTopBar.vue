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

    <!-- 右侧：收起面板 + 上下文 ring（hover 浮出文字） -->
    <div class="topbar-right">
      <button
        v-if="showPanel"
        type="button"
        class="topbar-btn"
        :class="{ 'topbar-btn--on': panelOpen }"
        :title="panelOpen ? '收起上下文面板' : '展开上下文面板（todoTree / 用量 / 压缩）'"
        @click="emit('toggle-panel')"
      >
        <Icon name="discover" :size="14" />
        <span class="topbar-hover-text">{{ panelOpen ? '收起面板' : '展开面板' }}</span>
      </button>

      <div v-if="showRing && max > 0" class="topbar-ring" :title="ringText">
        <ContextRing :used="used" :max="max" :size="18" />
        <span class="topbar-hover-text topbar-ring-text">{{ ringText }}</span>
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
  gap: 4px;
  pointer-events: auto;
  min-width: 0;
}

/* ---- 标题 / 面包屑 ---- */
.topbar-crumb {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 340px;
  padding: 2px 8px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  background: color-mix(in srgb, var(--bg) 78%, transparent);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
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
  font-size: 13px;
}

/* ---- 内联编辑输入 ---- */
.topbar-edit {
  width: 300px;
  max-width: 60vw;
  padding: 3px 8px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--primary);
  border-radius: var(--radius-md);
  outline: none;
}

/* ---- 右侧按钮 / ring ---- */
.topbar-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  font-size: 12px;
  color: var(--muted);
  background: color-mix(in srgb, var(--bg) 78%, transparent);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: color 0.15s, border-color 0.15s;
}

.topbar-btn:hover {
  color: var(--primary);
  border-color: var(--border);
}

.topbar-btn--on {
  color: var(--primary);
}

.topbar-ring {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  color: var(--muted);
  background: color-mix(in srgb, var(--bg) 78%, transparent);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  transition: border-color 0.15s;
}

.topbar-ring:hover {
  border-color: var(--border);
}

/* hover 浮出文字：默认隐藏，悬浮显示 */
.topbar-hover-text {
  max-width: 0;
  opacity: 0;
  overflow: hidden;
  white-space: nowrap;
  transition: max-width 0.22s ease, opacity 0.22s ease;
}

.topbar-btn:hover .topbar-hover-text,
.topbar-ring:hover .topbar-hover-text {
  max-width: 260px;
  opacity: 1;
}

.topbar-ring-text {
  font-size: 12px;
}
</style>