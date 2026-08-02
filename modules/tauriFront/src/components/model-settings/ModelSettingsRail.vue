<script setup lang="ts">
/**
 * ModelSettingsRail 模型设置二级栏目
 *
 * 当用户在左1栏（IconRail）点击"模型配置"图标时，左2栏从 HistoryRail 切换为本栏。
 * 本栏列出模型设置的子项，点击后切换主内容区的视图：
 * - providers：AI 服务商（编辑预设服务商 + 按能力类型填入具体模型）
 * - roles：服务模型（配置各使用场景的默认模型）
 *
 * 设计原则：
 * - 简约：与 HistoryRail 同宽（240px），纯文字 + 图标，无多余装饰
 * - 一致：复用 design tokens，hover/active 态与 HistoryItem 视觉对齐
 * - 动画：进入/离开使用淡入 + 轻微位移，避免突变
 */
import { Icon } from '../basic'

export type ModelSettingsView = 'providers' | 'roles'

const props = defineProps<{
  /** 当前选中的子项（'' 表示未选中任何子项，显示默认介绍页） */
  active: ModelSettingsView | ''
}>()

const emit = defineEmits<{
  (e: 'select', view: ModelSettingsView): void
}>()

const items: { key: ModelSettingsView; label: string; desc: string; icon: string }[] = [
  {
    key: 'providers',
    label: 'AI 服务商',
    desc: '编辑预设服务商 · 填入模型',
    icon: 'globe',
  },
  {
    key: 'roles',
    label: '服务模型',
    desc: '配置各场景默认模型',
    icon: 'robot',
  },
]

function select(view: ModelSettingsView) {
  emit('select', view)
}
</script>

<template>
  <nav class="ms-rail">
    <!-- 顶部标题 -->
    <header class="ms-rail-head">
      <span class="ms-rail-title">
        <Icon name="robot" :size="16" />
        模型设置
      </span>
    </header>

    <!-- 子项列表 -->
    <div class="ms-rail-list">
      <button
        v-for="item in items"
        :key="item.key"
        type="button"
        class="ms-rail-item"
        :class="{ active: props.active === item.key }"
        @click="select(item.key)"
      >
        <span class="ms-rail-item-glyph">
          <Icon :name="item.icon" :size="18" />
        </span>
        <span class="ms-rail-item-info">
          <span class="ms-rail-item-label">{{ item.label }}</span>
          <span class="ms-rail-item-desc">{{ item.desc }}</span>
        </span>
      </button>
    </div>

    <!-- 底部提示 -->
    <footer class="ms-rail-foot">
      <p class="ms-rail-hint">
        <Icon name="info" :size="12" />
        左侧图标栏切换回聊天
      </p>
    </footer>
  </nav>
</template>

<style scoped>
.ms-rail {
  display: flex;
  flex-direction: column;
  width: 240px;
  flex-shrink: 0;
  background: var(--bg-2);
  border-right: 1px solid var(--border);
  user-select: none;
  overflow: hidden;
}

.ms-rail-head {
  display: flex;
  align-items: center;
  padding: 14px 16px 10px;
  border-bottom: 1px solid var(--border);
}

.ms-rail-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  letter-spacing: 0.2px;
}

.ms-rail-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 10px;
  flex: 1;
  overflow-y: auto;
}

.ms-rail-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  outline: none;
  transition: background var(--duration-fast) var(--ease-standard),
    border-color var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.ms-rail-item:hover {
  background: var(--card);
}

.ms-rail-item.active {
  background: rgba(74, 126, 255, 0.12);
  border-color: rgba(74, 126, 255, 0.3);
}

.ms-rail-item:active {
  transform: scale(0.99);
}

.ms-rail-item-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-sm);
  background: var(--card-2);
  color: var(--muted);
  flex-shrink: 0;
  transition: color var(--duration-fast) var(--ease-standard),
    background var(--duration-fast) var(--ease-standard);
}

.ms-rail-item.active .ms-rail-item-glyph {
  color: var(--primary);
  background: rgba(74, 126, 255, 0.14);
}

.ms-rail-item-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.ms-rail-item-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ms-rail-item-desc {
  font-size: var(--fs-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ms-rail-foot {
  padding: 10px 14px;
  border-top: 1px solid var(--border);
}

.ms-rail-hint {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}
</style>
