<script setup lang="ts">
/**
 * SecondRailHost —— 左栏二宿主（HistoryRail 封装）
 *
 * 职责：
 * 1. 根据 useLayoutModes.rail2 决定展示完整二级栏还是隐藏
 * 2. 完整模式：在 AgentPoolRail / ModelSettingsRail / HistoryRail 三态间切换
 *    （原 App.vue 中的逻辑迁移至此，App.vue 不再膨胀）
 *
 * 对外透传 HistoryRail.refresh()（供 App.vue 会话标题更新后刷新列表）。
 */
import { ref } from 'vue'
import AgentPoolRail from './AgentPoolRail.vue'
import ModelSettingsRail, { type ModelSettingsView } from './model-settings/ModelSettingsRail.vue'
import HistoryRail from './HistoryRail.vue'
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

// 左2栏切换动画（expanded/hidden 切换共用）
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

// HistoryRail 实例引用（转发 refresh）
const historyRailRef = ref<{ refresh: () => void } | null>(null)
function refresh() {
  historyRailRef.value?.refresh()
}

defineExpose({ refresh })
</script>

<template>
  <!-- 外层容器：负责 rail2=hidden 隐藏动画（transform 滑出 + 延迟折叠宽度，减少 layout 重排） -->
  <div
    class="second-rail-host"
    :class="{ 'rail--hidden': modes.rail2 === 'hidden' }"
  >
    <!-- 展开模式：完整二级栏 -->
    <div class="second-rail-body">
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
  </div>
</template>

<style scoped>
/* 外层宿主：内容宽度由内部 body 决定；hidden 时折叠宽度释放空间 */
.second-rail-host {
  display: flex;
  flex-shrink: 0;
  height: 100%;
  transition: transform var(--duration-base) var(--ease-emphasized),
    opacity var(--duration-base) var(--ease-emphasized);
}

/* 隐藏模式：先 transform 滑出（合成器动画），再折叠宽度释放空间 */
.second-rail-host.rail--hidden {
  transform: translateX(-100%);
  opacity: 0;
  width: 0;
  overflow: hidden;
  transition: transform var(--duration-base) var(--ease-emphasized),
    opacity var(--duration-base) var(--ease-emphasized),
    width 0s var(--duration-base);
}

.second-rail-body {
  display: flex;
  flex-shrink: 0;
  min-width: 0;
  height: 100%;
}
</style>
