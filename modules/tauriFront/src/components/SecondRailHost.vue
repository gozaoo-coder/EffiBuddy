<script setup lang="ts">
/**
 * SecondRailHost —— 左栏二宿主（HistoryRail 展开 / 收起封装）
 *
 * 职责：
 * 1. 根据 useLayoutModes.rail2 决定展示完整二级栏还是收起窄条
 * 2. 完整模式：在 AgentPoolRail / ModelSettingsRail / HistoryRail 三态间切换
 *    （原 App.vue 中的逻辑迁移至此，App.vue 不再膨胀）
 * 3. 收起模式：窄条 + 展开按钮，点击恢复展开
 *
 * 对外透传 HistoryRail.refresh()（供 App.vue 会话标题更新后刷新列表）。
 */
import { ref } from 'vue'
import AgentPoolRail from './AgentPoolRail.vue'
import ModelSettingsRail, { type ModelSettingsView } from './model-settings/ModelSettingsRail.vue'
import HistoryRail from './HistoryRail.vue'
import Icon from './Icon.vue'
import { useLayoutModes } from '../composables/useLayoutModes'
import { useAnimeTransition } from '../composables/useAnimeTransition'

const props = withDefaults(defineProps<{
  /** 交流池模式（与 modelConfigOpen 互斥） */
  poolOpen?: boolean
  /** 模型配置模式 */
  modelConfigOpen?: boolean
  /** 模型配置二级子项 */
  modelSettingsView?: ModelSettingsView | ''
  /** 当前激活会话 id（HistoryRail 高亮） */
  activeChatConvId?: string | null
}>(), {
  poolOpen: false,
  modelConfigOpen: false,
  modelSettingsView: '',
  activeChatConvId: null,
})

const emit = defineEmits<{
  (e: 'select-conversation', id: string | null, title?: string | null): void
  (e: 'select-model-settings', view: ModelSettingsView): void
}>()

const { modes } = useLayoutModes()
const collapsed = ref(false)

// 左2栏切换动画（三态切换共用）
const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateX: [-12, 0],
    duration: 240,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateX: [0, -8],
    duration: 180,
    ease: 'inOut(2)',
  },
})


const isCollapsed = ref(false)
// 同步外部模态（loadModes 时）
function syncCollapsed() {
  isCollapsed.value = modes.value.rail2 === 'collapsed'
}
syncCollapsed()

// 收起窄条 + 展开按钮
function onExpand() {
  modes.value.rail2 = 'expanded'
  isCollapsed.value = false
}

// HistoryRail 实例引用（转发 refresh）
const historyRailRef = ref<{ refresh: () => void } | null>(null)
function refresh() {
  historyRailRef.value?.refresh()
}

defineExpose({ refresh })
</script>

<template>
  <!-- 收起 / 展开双态：共用一个 Transition（mode=out-in 先出后入） -->
  <Transition :css="false" @enter="onEnter" @leave="onLeave" mode="out-in">
    <!-- 收起模式：窄条 -->
    <aside v-if="isCollapsed" key="strip" class="second-rail-strip">
      <button type="button" class="strip-expand" title="展开左栏" @click="onExpand">
        <Icon name="chevron-right" :size="18" />
      </button>
      <span class="strip-caption">历史</span>
    </aside>

    <!-- 展开模式：完整二级栏 -->
    <div v-else key="body" class="second-rail-body">
      <Transition :css="false" @enter="onEnter" @leave="onLeave" mode="out-in">
        <AgentPoolRail
          v-if="props.poolOpen"
          key="pool"
        />
        <ModelSettingsRail
          v-else-if="props.modelConfigOpen"
          key="model-settings"
          :active="props.modelSettingsView"
          @select="(v) => emit('select-model-settings', v)"
        />
        <HistoryRail
          v-else
          key="history"
          ref="historyRailRef"
          :active-id="props.activeChatConvId ?? null"
          @select-conversation="(id, title) => emit('select-conversation', id, title)"
        />
      </Transition>
    </div>
  </Transition>
</template>

<style scoped>
.second-rail-strip {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 44px;
  flex-shrink: 0;
  background: var(--bg-2);
  border-right: 1px solid var(--border);
  padding: 10px 6px;
  user-select: none;
}

.strip-expand {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: background var(--duration-fast), color var(--duration-fast);
}

.strip-expand:hover {
  background: var(--card);
  color: var(--primary);
}

.strip-caption {
  margin-top: 12px;
  font-size: 11px;
  color: var(--muted);
  writing-mode: vertical-rl;
  letter-spacing: 2px;
  opacity: 0.7;
}

.second-rail-body {
  display: flex;
  flex-shrink: 0;
  min-width: 0;
  height: 100%;
}
</style>
