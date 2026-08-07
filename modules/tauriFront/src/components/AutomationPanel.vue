<script setup lang="ts">
/**
 * AutomationPanel 自动化面板容器
 * 将定时任务升级为「自动化」下的一个页签子类型，并新增：
 * - 定时任务（由原 SchedulePanel 迁移）
 * - Agent 流程（ComfyUI 风格可视化节点编辑器）
 * - Agent Team（智能体群聊，原 Agent Pool 的群聊化升级）
 * - 建立智能体（自定义 AgentDef）
 *
 * 外层复用 BindSheet side="right" 作为桌面端抽屉窗口；
 * 内部用页签子导航切换四个子视图，切换带轻量过渡动画。
 */
import { ref, watch } from 'vue'
import { BindSheet, SegmentedButton, type SegmentedOption } from './basic'
import ScheduleTasksView from './automation/ScheduleTasksView.vue'
import AgentFlowView from './automation/AgentFlowView.vue'
import AgentTeamView from './automation/AgentTeamView.vue'
import AgentDefView from './automation/AgentDefView.vue'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

// 子导航页签
type SubTab = 'schedule' | 'flow' | 'team' | 'def'
const activeTab = ref<SubTab>('schedule')

const tabOptions: SegmentedOption[] = [
  { label: '定时任务', value: 'schedule', icon: 'alarm' },
  { label: 'Agent流程', value: 'flow', icon: 'brain' },
  { label: 'Agent Team', value: 'team', icon: 'wechat' },
  { label: '建立智能体', value: 'def', icon: 'robot' },
]

function onTabChange(v: string | number) {
  activeTab.value = v as SubTab
}

// 每次打开抽屉时，若上次停留在某个子视图，重置回定时任务（更符合"定时任务为默认子类型"）
watch(
  () => props.open,
  (v) => {
    if (v) activeTab.value = 'schedule'
  },
)

function onClose() {
  emit('close')
}
</script>

<template>
  <BindSheet
    :visible="props.open"
    side="right"
    width="520px"
    title="自动化"
    @close="onClose"
  >
    <div class="auto-body">
      <!-- 子导航页签 -->
      <div class="auto-tabs">
        <SegmentedButton
          :model-value="activeTab"
          :options="tabOptions"
          size="md"
          block
          @update:model-value="onTabChange"
        />
      </div>

      <!-- 子视图（切换过渡） -->
      <div class="auto-content">
        <Transition name="subview" mode="out-in">
          <ScheduleTasksView v-if="activeTab === 'schedule'" key="schedule" />
          <AgentFlowView v-else-if="activeTab === 'flow'" key="flow" />
          <AgentTeamView v-else-if="activeTab === 'team'" key="team" />
          <AgentDefView v-else key="def" />
        </Transition>
      </div>
    </div>
  </BindSheet>
</template>

<style scoped>
.auto-body {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.auto-tabs {
  flex-shrink: 0;
  padding: 12px 16px 8px;
  border-bottom: 1px solid var(--border);
}

.auto-content {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

/* 子视图切换过渡：淡入 + 轻微上移 */
.subview-enter-active,
.subview-leave-active {
  transition: opacity var(--duration-fast) var(--ease-standard),
    transform var(--duration-fast) var(--ease-standard);
}

.subview-enter-from {
  opacity: 0;
  transform: translateY(8px);
}

.subview-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}
</style>