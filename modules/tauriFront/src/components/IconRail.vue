<script setup lang="ts">
/**
 * IconRail 第一栏导航（router 栏）
 *
 * 纯图标竖排，hover 时右侧浮出 tippy 风格提示条告知功能名称。
 * 从上到下：聊天 / 模型配置 / 自动化（含定时任务）/ 技能 / 插件 / 更多
 *
 * 「更多」为底部弹出菜单（ClawHub / 设备管理 / 设置）。
 */
import { ref } from 'vue'
import Icon from './Icon.vue'
import { Menu, type MenuItemOption } from './basic'

export type RailView = 'chat' | 'model-config' | 'automation' | 'skills' | 'plugins'

const props = defineProps<{
  /** 当前激活的视图（用于高亮对应图标） */
  active?: RailView | ''
}>()

const emit = defineEmits<{
  (e: 'select', view: RailView): void
  (e: 'open-clawhub'): void
  (e: 'open-device'): void
  (e: 'open-settings'): void
}>()

// 主栏图标（纯 icon，无文字）
const railItems: { key: RailView; label: string; icon: string }[] = [
  { key: 'chat', label: '聊天', icon: 'chat' },
  { key: 'model-config', label: '模型配置', icon: 'robot' },
  { key: 'automation', label: '自动化', icon: 'alarm' },
  { key: 'skills', label: '技能', icon: 'bolt' },
  { key: 'plugins', label: '插件', icon: 'puzzle' },
]

// 「更多」弹出菜单
const moreMenuVisible = ref(false)
const moreBtnRef = ref<HTMLElement | null>(null)

const moreItems: MenuItemOption[] = [
  { key: 'clawhub', label: 'ClawHub 技能市场', icon: 'globe' },
  { key: 'device', label: '设备管理', icon: 'device' },
  { key: 'settings', label: '设置', icon: 'settings' },
]

function onMoreSelect(item: MenuItemOption) {
  moreMenuVisible.value = false
  switch (item.key) {
    case 'clawhub':
      emit('open-clawhub')
      break
    case 'device':
      emit('open-device')
      break
    case 'settings':
      emit('open-settings')
      break
  }
}

function select(view: RailView) {
  emit('select', view)
}
</script>

<template>
  <nav class="icon-rail">
    <!-- 主栏图标 -->
    <div class="rail-group">
      <button
        v-for="item in railItems"
        :key="item.key"
        type="button"
        class="rail-btn"
        :class="{ active: props.active === item.key }"
        @click="select(item.key)"
      >
        <Icon :name="item.icon" :size="21" />
        <span class="rail-tip">{{ item.label }}</span>
      </button>
    </div>

    <!-- 底部：更多 -->
    <div class="rail-group rail-group--bottom">
      <button
        ref="moreBtnRef"
        type="button"
        class="rail-btn"
        :class="{ active: moreMenuVisible }"
        @click="moreMenuVisible = true"
      >
        <Icon name="more-horizontal" :size="21" />
        <span class="rail-tip">更多</span>
      </button>
    </div>

    <!-- 更多菜单 -->
    <Menu
      v-model:visible="moreMenuVisible"
      :items="moreItems"
      :trigger-ref="moreBtnRef"
      placement="right-start"
      :min-width="176"
      @select="onMoreSelect"
    />
  </nav>
</template>

<style scoped>
.icon-rail {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  width: 56px;
  flex-shrink: 0;
  background: var(--bg-2);
  border-right: 1px solid var(--border);
  padding: 10px 8px;
  user-select: none;
}

.rail-group {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}

.rail-group--bottom {
  gap: 4px;
}

/* 图标按钮 */
.rail-btn {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  padding: 0;
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  outline: none;
  transition: background var(--duration-fast) var(--ease-standard),
    color var(--duration-fast) var(--ease-standard);
}

.rail-btn:hover {
  background: var(--card);
  color: var(--text);
}

.rail-btn.active {
  background: rgba(74, 126, 255, 0.14);
  color: var(--primary);
}

.rail-btn:active {
  transform: scale(0.94);
}

/* tippy 风格提示条：hover 时在右侧浮出 */
.rail-tip {
  position: absolute;
  left: calc(100% + 10px);
  top: 50%;
  transform: translateY(-50%) translateX(-4px);
  padding: 5px 11px;
  background: var(--card-2);
  color: var(--text);
  font-size: 12px;
  font-weight: 500;
  line-height: 1.4;
  white-space: nowrap;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow);
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  z-index: calc(var(--z-menu) + 1);
  transition: opacity var(--duration-fast) var(--ease-decelerated),
    transform var(--duration-fast) var(--ease-decelerated),
    visibility var(--duration-fast) var(--ease-decelerated);
}

/* 小三角箭头 */
.rail-tip::before {
  content: '';
  position: absolute;
  left: -4px;
  top: 50%;
  width: 8px;
  height: 8px;
  background: var(--card-2);
  border-left: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
  transform: translateY(-50%) rotate(45deg);
}

.rail-btn:hover .rail-tip {
  opacity: 1;
  visibility: visible;
  transform: translateY(-50%) translateX(0);
}
</style>
