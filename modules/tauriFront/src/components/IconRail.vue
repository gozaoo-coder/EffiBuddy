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

export type RailView = 'chat' | 'model-config' | 'automation' | 'skills' | 'plugins' | 'pool'

const props = withDefaults(defineProps<{
  /** 当前激活的视图（用于高亮对应图标） */
  active?: RailView | ''
  /** P2P 待配对请求计数（>0 时在 P2P 按钮上显示气泡角标） */
  pendingPairCount?: number
  /** 交流池活跃条目数（>0 时在交流池按钮上显示气泡角标） */
  poolActiveCount?: number
}>(), {
  pendingPairCount: 0,
  poolActiveCount: 0,
})

const emit = defineEmits<{
  (e: 'select', view: RailView): void
  (e: 'open-clawhub'): void
  (e: 'open-device'): void
  (e: 'open-p2p'): void
  (e: 'open-settings'): void
  (e: 'open-asr', kind: 'asr-stream' | 'asr-upload' | 'asr-history'): void
}>()

// 主栏图标（纯 icon，无文字）
const railItems: { key: RailView; label: string; icon: string }[] = [
  { key: 'chat', label: '聊天', icon: 'chat' },
  { key: 'pool', label: 'Agent 交流池', icon: 'merge' },
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

// ASR 弹出菜单
const asrMenuVisible = ref(false)
const asrBtnRef = ref<HTMLElement | null>(null)

const asrItems: MenuItemOption[] = [
  { key: 'asr-stream', label: '流式录入', icon: 'mic' },
  { key: 'asr-upload', label: '文件转写', icon: 'attachment' },
  { key: 'asr-history', label: '历史记录', icon: 'clock' },
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

function onAsrSelect(item: MenuItemOption) {
  asrMenuVisible.value = false
  emit('open-asr', item.key as 'asr-stream' | 'asr-upload' | 'asr-history')
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
        <!-- 交流池活跃条目角标（仅 pool 项；>0 时显示） -->
        <span
          v-if="item.key === 'pool' && props.poolActiveCount > 0"
          :key="props.poolActiveCount"
          class="pool-badge"
        >
          {{ props.poolActiveCount > 99 ? '99+' : props.poolActiveCount }}
        </span>
      </button>

      <!-- ASR 语音转写（弹出菜单选择录入/上传/历史） -->
      <button
        ref="asrBtnRef"
        type="button"
        class="rail-btn"
        :class="{ active: asrMenuVisible }"
        @click="asrMenuVisible = true"
      >
        <Icon name="mic" :size="21" />
        <span class="rail-tip">语音转写</span>
      </button>
    </div>

    <!-- 底部：P2P 设备 + 更多 -->
    <div class="rail-group rail-group--bottom">
      <!-- P2P 设备入口（含待配对请求气泡角标） -->
      <button
        type="button"
        class="rail-btn"
        @click="emit('open-p2p')"
      >
        <Icon name="device" :size="21" />
        <span class="rail-tip">P2P 设备</span>
        <span
          v-if="props.pendingPairCount > 0"
          :key="props.pendingPairCount"
          class="p2p-badge"
        >
          {{ props.pendingPairCount > 99 ? '99+' : props.pendingPairCount }}
        </span>
      </button>

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
