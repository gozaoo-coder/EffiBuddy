<script setup lang="ts">
/**
 * IconRailSettings —— 「修改侧栏icon」设置面板
 *
 * 由左栏一底部「更多」菜单的「修改侧栏icon」入口打开，能力：
 * 1. 切换左栏一模态（纯图标 / 图标+文字）→ useLayoutModes
 * 2. 逐项显隐开关（是否在左栏一显示该项）→ useRailItems.setVisible
 * 3. 逐项自定义图标（从内置语义图标库中挑选）→ useRailItems.setIcon
 * 4. 一键重置所有侧栏偏好
 *
 * 自包含模态浮层（不依赖 Dialog/BindSheet），设计 token 与全局一致。
 */
  import { ref } from 'vue'
  import { SegmentedButton, Switch, Icon, useToast } from './basic'
  import { useLayoutModes } from '../composables/useLayoutModes'
  import { useRailItems } from '../composables/useRailItems'

const props = withDefaults(defineProps<{ visible: boolean }>(), { visible: false })
const emit = defineEmits<{ (e: 'update:visible', v: boolean): void; (e: 'close'): void }>()

const { modes } = useLayoutModes()
const { editableItems, isVisible, setVisible, iconFor, setIcon, resetItemPrefs } = useRailItems()
const { toast } = useToast()

const rail1Options = [
  { value: 'icon', label: '纯图标' },
  { value: 'icon-text', label: '图标+文字' },
]

/** 当前展开图标选择器的项 key */
const iconPickerKey = ref<string | null>(null)

/** 可挑选的语义图标库（iconMap 命中集合的常用子集） */
const ICON_PALETTE = [
  'chat', 'merge', 'robot', 'alarm', 'bolt', 'puzzle',
  'mic', 'device', 'settings', 'search', 'folder', 'book',
  'clock', 'star', 'pin', 'sparkles', 'home', 'cloud',
  'tool', 'image', 'globe', 'message', 'brain', 'keyboard',
  'power', 'palette', 'view', 'history', 'discover',
]

function toggleIconPicker(key: string) {
  iconPickerKey.value = iconPickerKey.value === key ? null : key
}

function applyIcon(key: string, icon: string) {
  setIcon(key, icon)
  iconPickerKey.value = null
}

function onReset() {
  resetItemPrefs()
  toast({ content: '已重置侧栏偏好', type: 'success' })
}

function close() {
  emit('update:visible', false)
  emit('close')
}

function onOverlayClick() {
  close()
}
</script>

<template>
  <Teleport to="body">
    <Transition name="rail-settings">
      <div v-if="props.visible" class="rail-settings-overlay" @click.self="onOverlayClick">
        <div class="rail-settings-panel">
          <!-- 头部 -->
          <header class="rs-head">
            <div class="rs-head-title">
              <span class="rs-head-icon"><Icon name="palette" :size="18" /></span>
              <span>修改侧栏图标</span>
            </div>
            <button type="button" class="rs-close" title="关闭" @click="close">
              <Icon name="close" :size="16" />
            </button>
          </header>

          <div class="rs-body">
            <!-- 模态切换 -->
            <section class="rs-section">
              <h4 class="rs-section-title">侧栏样式</h4>
              <SegmentedButton
                :model-value="modes.rail1"
                :options="rail1Options"
                block
                @update:model-value="modes.rail1 = $event as 'icon' | 'icon-text'"
              />
              <p class="rs-hint">图标模式：紧凑竖排，hover 显示提示；图标+文字：常驻文字更直观</p>
            </section>

            <!-- 显隐与图标 -->
            <section class="rs-section">
              <h4 class="rs-section-title">显示项</h4>
              <div class="rs-items">
                <div v-for="item in editableItems" :key="item.key" class="rs-item">
                  <span class="rs-item-icon">
                    <Icon :name="iconFor(item.key)" :size="17" />
                  </span>
                  <span class="rs-item-label" :title="item.label">{{ item.label }}</span>
                  <span v-if="!item.builtin" class="rs-item-tag">插件</span>
                  <button
                    type="button"
                    class="rs-item-edit"
                    :title="'修改图标：' + item.label"
                    @click="toggleIconPicker(item.key)"
                  >
                    <Icon name="edit" :size="14" />
                  </button>
                  <Switch
                    :model-value="isVisible(item.key)"
                    :disabled="item.fixed"
                    size="sm"
                    @update:model-value="(v) => setVisible(item.key, v)"
                  />
                </div>
              </div>

              <!-- 图标选择器（内联展开） -->
              <div v-if="iconPickerKey" class="rs-icon-picker">
                <div class="rs-picker-grid">
                  <button
                    v-for="ic in ICON_PALETTE"
                    :key="ic"
                    type="button"
                    class="rs-picker-opt"
                    :class="{ active: iconFor(iconPickerKey) === ic }"
                    :title="ic"
                    @click="applyIcon(iconPickerKey, ic)"
                  >
                    <Icon :name="ic" :size="17" />
                  </button>
                </div>
                <button
                  type="button"
                  class="rs-picker-reset"
                  @click="applyIcon(iconPickerKey, '')"
                >
                  恢复默认图标
                </button>
              </div>
            </section>
          </div>

          <!-- 底部 -->
          <footer class="rs-foot">
            <button type="button" class="rs-btn rs-btn--ghost" @click="onReset">重置全部</button>
            <button type="button" class="rs-btn rs-btn--primary" @click="close">完成</button>
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.rail-settings-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--z-dialog);
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(2px);
}

.rail-settings-panel {
  width: 420px;
  max-width: calc(100vw - 48px);
  max-height: calc(100vh - 96px);
  display: flex;
  flex-direction: column;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

/* 头部 */
.rs-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.rs-head-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text);
}

.rs-head-icon {
  display: inline-flex;
  color: var(--primary);
}

.rs-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  transition: background var(--duration-fast), color var(--duration-fast);
}

.rs-close:hover {
  background: var(--card);
  color: var(--text);
}

/* 主体 */
.rs-body {
  padding: 16px 18px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.rs-section-title {
  margin: 0 0 10px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--muted);
  letter-spacing: 0.3px;
}

.rs-hint {
  margin: 8px 0 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

/* 项列表 */
.rs-items {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.rs-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  transition: background var(--duration-fast);
}

.rs-item:hover {
  background: var(--card);
}

.rs-item-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  background: var(--card);
  color: var(--text);
  flex-shrink: 0;
}

.rs-item-label {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-base);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rs-item-tag {
  flex-shrink: 0;
  padding: 1px 6px;
  font-size: var(--fs-xs);
  color: var(--primary);
  background: rgba(74, 126, 255, 0.12);
  border-radius: var(--radius-full);
}

.rs-item-edit {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  flex-shrink: 0;
  transition: background var(--duration-fast), color var(--duration-fast);
}

.rs-item-edit:hover {
  background: var(--card-2);
  color: var(--primary);
}

/* 图标选择器 */
.rs-icon-picker {
  margin-top: 8px;
  padding: 12px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.rs-picker-grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 6px;
}

.rs-picker-opt {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  aspect-ratio: 1;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  transition: background var(--duration-fast), color var(--duration-fast);
}

.rs-picker-opt:hover {
  background: var(--card-2);
  color: var(--text);
}

.rs-picker-opt.active {
  background: rgba(74, 126, 255, 0.16);
  color: var(--primary);
}

.rs-picker-reset {
  margin-top: 10px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--primary);
  font-size: var(--fs-sm);
  cursor: pointer;
}

/* 底部 */
.rs-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 12px 18px;
  border-top: 1px solid var(--border);
  flex-shrink: 0;
}

.rs-btn {
  padding: 7px 16px;
  border-radius: var(--radius-sm);
  font-size: var(--fs-base);
  cursor: pointer;
  transition: background var(--duration-fast), color var(--duration-fast),
    opacity var(--duration-fast);
}

.rs-btn--ghost {
  background: transparent;
  color: var(--muted);
}

.rs-btn--ghost:hover {
  background: var(--card);
  color: var(--text);
}

.rs-btn--primary {
  background: var(--primary);
  color: #fff;
}

.rs-btn--primary:hover {
  opacity: 0.9;
}

/* 过渡动画 */
.rail-settings-enter-active,
.rail-settings-leave-active {
  transition: opacity var(--duration-base) var(--ease-standard);
}

.rail-settings-enter-active .rail-settings-panel,
.rail-settings-leave-active .rail-settings-panel {
  transition: transform var(--duration-base) var(--ease-emphasized),
    opacity var(--duration-base) var(--ease-standard);
}

.rail-settings-enter-from,
.rail-settings-leave-to {
  opacity: 0;
}

.rail-settings-enter-from .rail-settings-panel,
.rail-settings-leave-to .rail-settings-panel {
  transform: scale(0.96) translateY(8px);
  opacity: 0;
}
</style>
