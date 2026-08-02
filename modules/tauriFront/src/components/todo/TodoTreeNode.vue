<script setup lang="ts">
/**
 * TodoTreeNode —— 递归渲染的 todoTree 树节点
 *
 * 支持任意深度：每个节点都渲染状态按钮 / 内容 / 优先级 / 操作（添加子任务、编辑、删除），
 * 并通过自身递归把 children 逐层展开，配合 CSS 左侧虚线形成树状连接线。
 *
 * 两种形态：
 * - 可编辑（右栏概览面板）：完整操作按钮 + 状态/优先级可点击切换
 * - 只读（长程任务气泡）：隐藏操作按钮、状态仅展示
 *
 * 编辑态（editing）与扁平任务数组（items）由父级以「引用」方式传入，
 * 组件内直接就地更新同一份响应式数据，父级持有同一引用，无需逐层 emit。
 */
import { computed } from 'vue'
import { Icon, IconButton } from '../basic'
import type { TodoItem, TodoNode, TodoPriority, TodoStatus } from '../../types'

defineOptions({ name: 'TodoTreeNode' })

const props = withDefaults(
  defineProps<{
    /** 当前树节点 */
    node: TodoNode
    /** 当前层深（根=0），递归时 +1 */
    depth?: number
    /** 扁平任务数组（响应式引用，就地更新触发父级树重算） */
    items: TodoItem[]
    /** 全局编辑态映射（key: edit-<id> / add-child-<parentId>） */
    editing: Record<string, TodoItem>
    /** 生成新任务 id */
    genId: () => string
    /** 任意变更后回调（父级负责持久化到后端） */
    onChanged: () => void
    /** 只读展示（长程任务气泡用） */
    readonly?: boolean
  }>(),
  { depth: 0, readonly: false },
)

const statusIcon = computed(() =>
  props.node.status === 'completed'
    ? 'check'
    : props.node.status === 'in_progress'
      ? 'bolt'
      : 'clock',
)

function cycleStatus(item: TodoItem) {
  if (props.readonly) return
  const next: TodoStatus =
    item.status === 'pending'
      ? 'in_progress'
      : item.status === 'in_progress'
        ? 'completed'
        : 'pending'
  const idx = props.items.findIndex((t) => t.id === item.id)
  if (idx >= 0) {
    props.items[idx] = { ...props.items[idx], status: next }
    props.onChanged()
  }
}

function cyclePriority(item: TodoItem) {
  if (props.readonly) return
  const next: TodoPriority =
    item.priority === 'high' ? 'medium' : item.priority === 'medium' ? 'low' : 'high'
  const idx = props.items.findIndex((t) => t.id === item.id)
  if (idx >= 0) {
    props.items[idx] = { ...props.items[idx], priority: next }
    props.onChanged()
  }
}

function beginAddChild() {
  props.editing[`add-child-${props.node.id}`] = {
    id: '',
    content: '',
    priority: 'medium',
    status: 'pending',
    parent_id: props.node.id,
  }
}

function beginEdit() {
  const src = props.items.find((t) => t.id === props.node.id)
  props.editing[`edit-${props.node.id}`] = {
    id: props.node.id,
    content: props.node.content,
    priority: props.node.priority,
    status: props.node.status,
    parent_id: src?.parent_id ?? null,
  }
}

function cancelEdit(key: string) {
  delete props.editing[key]
}

function confirmAdd(key: string) {
  const draft = props.editing[key]
  if (!draft || !draft.content.trim()) return
  props.items.push({
    ...draft,
    id: draft.id || props.genId(),
    content: draft.content.trim(),
  })
  delete props.editing[key]
  props.onChanged()
}

function confirmEdit(key: string) {
  const draft = props.editing[key]
  if (!draft) return
  const idx = props.items.findIndex((t) => t.id === draft.id)
  if (idx >= 0) props.items[idx] = { ...draft, content: draft.content.trim() }
  delete props.editing[key]
  props.onChanged()
}

function removeItem(id: string) {
  // 保留原数组引用（props.items 指向父级响应式数组），仅替换内容
  props.items.splice(0, props.items.length, ...props.items.filter((t) => t.id !== id && t.parent_id !== id))
  props.onChanged()
}
</script>

<template>
  <div class="ttn">
    <div class="ttn-row">
      <span class="ttn-status" :class="node.status" :title="readonly ? '' : node.status">
        <Icon :name="statusIcon" :size="readonly ? 12 : 14" />
      </span>
      <span
        class="ttn-content"
        :class="{ done: node.status === 'completed' }"
        :title="node.content"
        @dblclick="!readonly && beginEdit()"
      >
        {{ node.content }}
      </span>
      <span
        v-if="!readonly"
        class="ttn-priority"
        :class="node.priority"
        title="点击切换优先级"
        @click="cyclePriority({ id: node.id, content: node.content, priority: node.priority, status: node.status, parent_id: undefined })"
      >
        {{ node.priority === 'high' ? '高' : node.priority === 'medium' ? '中' : '低' }}
      </span>
      <div v-if="!readonly" class="ttn-ops">
        <IconButton size="sm" icon="plus" title="添加子任务" @click="beginAddChild" />
        <IconButton size="sm" icon="edit" title="编辑" @click="beginEdit" />
        <IconButton size="sm" icon="delete" title="删除" @click="removeItem(node.id)" />
      </div>
    </div>

    <!-- 节点编辑框 -->
    <div v-if="!readonly && editing[`edit-${node.id}`]" class="ttn-add ttn-add--edit">
      <input
        v-model="editing[`edit-${node.id}`].content"
        class="ttn-input"
        placeholder="任务内容…"
        @keydown.enter="confirmEdit(`edit-${node.id}`)"
        @keydown.esc="cancelEdit(`edit-${node.id}`)"
      />
      <button type="button" class="ttn-ok" title="确认" @click="confirmEdit(`edit-${node.id}`)">
        <Icon name="check" :size="14" />
      </button>
      <button type="button" class="ttn-x" title="取消" @click="cancelEdit(`edit-${node.id}`)">
        <Icon name="close" :size="14" />
      </button>
    </div>

    <!-- 添加子任务框 -->
    <div v-if="!readonly && editing[`add-child-${node.id}`]" class="ttn-add ttn-add--child">
      <input
        v-model="editing[`add-child-${node.id}`].content"
        class="ttn-input"
        placeholder="子任务…"
        @keydown.enter="confirmAdd(`add-child-${node.id}`)"
        @keydown.esc="cancelEdit(`add-child-${node.id}`)"
      />
      <button type="button" class="ttn-ok" title="确认" @click="confirmAdd(`add-child-${node.id}`)">
        <Icon name="check" :size="14" />
      </button>
      <button type="button" class="ttn-x" title="取消" @click="cancelEdit(`add-child-${node.id}`)">
        <Icon name="close" :size="14" />
      </button>
    </div>

    <!-- 递归子节点 -->
    <div v-if="node.children.length" class="ttn-children">
      <TodoTreeNode
        v-for="child in node.children"
        :key="child.id"
        :node="child"
        :depth="depth + 1"
        :items="items"
        :editing="editing"
        :gen-id="genId"
        :on-changed="onChanged"
        :readonly="readonly"
      />
    </div>
  </div>
</template>

<style scoped>
.ttn {
  display: flex;
  flex-direction: column;
}
.ttn-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 2px;
  border-radius: var(--radius-xs);
}
.ttn-row:hover {
  background: var(--hover);
}
.ttn-status {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  flex-shrink: 0;
  color: var(--muted);
}
.ttn-status.completed {
  color: var(--success);
}
.ttn-status.in_progress {
  color: var(--primary);
}
.ttn-content {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-sm);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ttn-content.done {
  color: var(--muted);
  text-decoration: line-through;
}
.ttn-priority {
  font-size: var(--fs-xs);
  padding: 1px 6px;
  border-radius: var(--radius-full);
  cursor: pointer;
  flex-shrink: 0;
}
.ttn-priority.high {
  background: color-mix(in srgb, #f5222d 14%, var(--card));
  color: #f5222d;
}
.ttn-priority.medium {
  background: color-mix(in srgb, #fa8c16 14%, var(--card));
  color: #fa8c16;
}
.ttn-priority.low {
  background: color-mix(in srgb, #52c41a 14%, var(--card));
  color: #52c41a;
}
.ttn-ops {
  display: none;
  gap: 2px;
  flex-shrink: 0;
}
.ttn-row:hover .ttn-ops {
  display: inline-flex;
}
/* 树状连接线：每层递归都带左侧虚线 + 缩进 */
.ttn-children {
  margin-left: 13px;
  padding-left: 8px;
  border-left: 1px dashed var(--border);
}
.ttn-add {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 2px;
}
.ttn-add--edit {
  padding-left: 26px;
}
.ttn-add--child {
  padding-left: 40px;
}
.ttn-input {
  flex: 1;
  min-width: 0;
  height: 24px;
  padding: 0 8px;
  font-size: var(--fs-sm);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  outline: none;
}
.ttn-input:focus {
  border-color: var(--primary);
}
.ttn-ok,
.ttn-x {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  cursor: pointer;
  color: var(--muted);
}
.ttn-ok:hover {
  color: var(--success);
  background: var(--hover);
}
.ttn-x:hover {
  color: var(--danger);
  background: var(--hover);
}
</style>
