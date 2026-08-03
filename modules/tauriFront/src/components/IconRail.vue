<script setup lang="ts">
/**
 * IconRail 第一栏导航（router 栏）
 *
 * 双模态（useLayoutModes 驱动）：
 * - `icon`：纯图标竖排（默认），hover 右侧浮出 tippy 提示条
 * - `icon-text`：图标 + 文字常驻，更直观
 *
 * 数据驱动（useRailItems 注册表）：
 * - 内置项（聊天 / 交流池 / 模型配置 / 自动化 / 技能 / 插件 / ASR / P2P / 更多）
 * - 插件贡献的 rail 按钮（usePluginContributions → setPluginContributions 注入）
 * - 用户可在「更多」菜单 →「修改侧栏icon」面板调整显隐与图标
 *
 * 事件：select / open-plugin-page / open-plugin-command / open-clawhub /
 *       open-device / open-p2p / open-settings / open-asr
 */
import { ref, onMounted, computed } from 'vue'
import Icon from './Icon.vue'
import { Menu, type MenuItemOption } from './basic'
import IconRailSettings from './IconRailSettings.vue'
import { useLayoutModes } from '../composables/useLayoutModes'
import { useRailItems } from '../composables/useRailItems'
import { usePluginContributions } from '../composables/usePluginContributions'

export type RailView = 'chat' | 'model-config' | 'automation' | 'skills' | 'plugins' | 'pool'

const props = withDefaults(defineProps<{
  /** 当前激活的视图（用于高亮对应图标） */
  active?: RailView | ''
  /** P2P 待配对请求计数（>0 时在 P2P 按钮上显示气泡角标） */
  pendingPairCount?: number
  /** 交流池活跃条目数（>0 时在交流池按钮上显示气泡角标） */
  poolActiveCount?: number
}>(), {
  active: '',
  pendingPairCount: 0,
  poolActiveCount: 0,
})

const emit = defineEmits<{
  (e: 'select', view: RailView): void
  (e: 'open-plugin-page', pageId: string): void
  (e: 'open-plugin-command', commandId: string): void
  (e: 'open-clawhub'): void
  (e: 'open-device'): void
  (e: 'open-p2p'): void
  (e: 'open-settings'): void
  (e: 'open-asr', kind: 'asr-stream' | 'asr-upload' | 'asr-history'): void
}>()

const { modes } = useLayoutModes()
const { mainItems, bottomItems, iconFor } = useRailItems()
const { railButtons, install: installPluginContributions } = usePluginContributions()
const { setPluginContributions } = useRailItems()

/** 当前是否显示文字（icon-text 模式） */
const showText = computed(() => modes.value.rail1 === 'icon-text')

/** 插件 rail 贡献 → 注册表（响应式注入） */
onMounted(() => {
  void installPluginContributions()
})

// 插件按钮点击：open-page → 打开页签；command → 触发命令
function onPluginClick(item: { pageId?: string; command?: string }) {
  if (item.pageId) emit('open-plugin-page', item.pageId)
  else if (item.command) emit('open-plugin-command', item.command)
}

// ============= 「更多」弹出菜单 =============
const moreMenuVisible = ref(false)
const moreBtnRef = ref<HTMLElement | null>(null)

const moreItems: MenuItemOption[] = [
  { key: 'rail-settings', label: '修改侧栏icon', icon: 'palette' },
  { key: 'clawhub', label: 'ClawHub 技能市场', icon: 'globe', divided: true },
  { key: 'device', label: '设备管理', icon: 'device' },
  { key: 'settings', label: '设置', icon: 'settings' },
]

const railSettingsVisible = ref(false)

function onMoreSelect(item: MenuItemOption) {
  moreMenuVisible.value = false
  switch (item.key) {
    case 'rail-settings':
      railSettingsVisible.value = true
      break
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

// ============= ASR 弹出菜单 =============
const asrMenuVisible = ref(false)
const asrBtnRef = ref<HTMLElement | null>(null)

const asrItems: MenuItemOption[] = [
  { key: 'asr-stream', label: '流式录入', icon: 'mic' },
  { key: 'asr-upload', label: '文件转写', icon: 'attachment' },
  { key: 'asr-history', label: '历史记录', icon: 'clock' },
]

function onAsrSelect(item: MenuItemOption) {
  asrMenuVisible.value = false
  emit('open-asr', item.key as 'asr-stream' | 'asr-upload' | 'asr-history')
}

function select(view: RailView) {
  emit('select', view)
}

// 插件贡献变化 → 同步到 rail 注册表（含用户偏好保留）
import { watch } from 'vue'
watch(
  () => railButtons.value,
  (btns) => setPluginContributions(btns),
  { immediate: true },
)
</script>

<template>
  <nav class="icon-rail" :class="modes.rail1 === 'icon-text' ? 'rail--icon-text' : 'rail--icon'">
    <!-- 主区 -->
    <div class="rail-group">
      <button
        v-for="item in mainItems"
        :key="item.key"
        type="button"
        class="rail-btn"
        :class="{ active: props.active === item.value }"
        :title="showText ? undefined : item.label"
        @click="
          item.kind === 'view'
            ? select(item.value as RailView)
            : item.kind === 'asr'
              ? (asrMenuVisible = true)
              : onPluginClick(item)
        "
      >
        <Icon :name="iconFor(item.key)" :size="21" />
        <span v-if="showText" class="rail-text">{{ item.label }}</span>
        <span v-else class="rail-tip">{{ item.label }}</span>
        <!-- 交流池活跃条目角标（仅 pool 项；>0 时显示） -->
        <span
          v-if="item.key === 'pool' && props.poolActiveCount > 0"
          :key="props.poolActiveCount"
          class="pool-badge"
        >
          {{ props.poolActiveCount > 99 ? '99+' : props.poolActiveCount }}
        </span>
      </button>
    </div>

    <!-- 底部：P2P 设备 + 更多 -->
    <div class="rail-group rail-group--bottom">
      <template v-for="item in bottomItems" :key="item.key">
        <button
          v-if="item.kind === 'p2p'"
          type="button"
          class="rail-btn"
          :title="showText ? undefined : item.label"
          @click="emit('open-p2p')"
        >
          <Icon :name="iconFor(item.key)" :size="21" />
          <span v-if="showText" class="rail-text">{{ item.label }}</span>
          <span v-else class="rail-tip">{{ item.label }}</span>
          <span
            v-if="props.pendingPairCount > 0"
            :key="props.pendingPairCount"
            class="p2p-badge"
          >
            {{ props.pendingPairCount > 99 ? '99+' : props.pendingPairCount }}
          </span>
        </button>

        <button
          v-else-if="item.kind === 'more'"
          ref="moreBtnRef"
          type="button"
          class="rail-btn"
          :class="{ active: moreMenuVisible }"
          :title="showText ? undefined : item.label"
          @click="moreMenuVisible = true"
        >
          <Icon :name="iconFor(item.key)" :size="21" />
          <span v-if="showText" class="rail-text">{{ item.label }}</span>
          <span v-else class="rail-tip">{{ item.label }}</span>
        </button>
      </template>
    </div>

    <!-- ASR 菜单 -->
    <Menu
      v-model:visible="asrMenuVisible"
      :items="asrItems"
      :trigger-ref="asrBtnRef"
      placement="right-start"
      :min-width="176"
      @select="onAsrSelect"
    />

    <!-- 更多菜单 -->
    <Menu
      v-model:visible="moreMenuVisible"
      :items="moreItems"
      :trigger-ref="moreBtnRef"
      placement="right-start"
      :min-width="176"
      @select="onMoreSelect"
    />

    <!-- 修改侧栏icon 设置面板 -->
    <IconRailSettings v-model:visible="railSettingsVisible" />
  </nav>
</template>

<style scoped>
/* 纯图标模式：窄栏 */
.rail--icon {
  width: 56px;
}

/* 图标+文字模式：宽栏 */
.rail--icon-text {
  width: 178px;
}

.icon-rail {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  flex-shrink: 0;
  background: var(--bg-2);
  border-right: 1px solid var(--border);
  padding: 10px 8px;
  user-select: none;
  transition: width var(--duration-base) var(--ease-emphasized);
}

.rail-group {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 6px;
}

.rail-group--bottom {
  gap: 4px;
}

/* 图标按钮 */
.rail-btn {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  width: 100%;
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

/* 图标+文字模式：文字常驻 */
.rail-text {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: 500;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* tippy 风格提示条：仅纯图标模式生效（hover 时在右侧浮出） */
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

/* P2P 待配对请求气泡角标（pointer-events: none 使点击穿透到父按钮，亦触发 open-p2p） */
.p2p-badge {
  position: absolute;
  top: 2px;
  right: 2px;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-full);
  background: #f56565;
  color: white;
  font-size: 10px;
  font-weight: 600;
  line-height: 16px;
  text-align: center;
  border: 2px solid var(--bg-2);
  animation: badge-pop 300ms var(--ease-decelerated);
  pointer-events: none;
}

/* 交流池活跃条目角标（与 p2p-badge 同样 pointer-events: none，点击穿透到父按钮） */
.pool-badge {
  position: absolute;
  top: 2px;
  right: 2px;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-full);
  background: var(--primary, #4a7eff);
  color: white;
  font-size: 10px;
  font-weight: 600;
  line-height: 16px;
  text-align: center;
  border: 2px solid var(--bg-2);
  animation: badge-pop 300ms var(--ease-decelerated);
  pointer-events: none;
}

@keyframes badge-pop {
  0% { transform: scale(0); }
  60% { transform: scale(1.15); }
  100% { transform: scale(1); }
}
</style>
