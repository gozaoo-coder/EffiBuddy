<script setup lang="ts">
/**
 * SubAgentTab —— sub-agent 页签视图容器
 *
 * 子 agent 会话以独立页签形式展示（由 useSubAgentStore 在子 agent 启动时
 * 后台自动打开）。本组件包装 SubAgentWindow，仅透传 sessionId。
 */
import SubAgentWindow from '../SubAgentWindow.vue'
import type { TabItem } from '../../types'

defineOptions({ name: 'SubAgentTab' })

const props = defineProps<{
  tab: TabItem
}>()
</script>

<template>
  <div class="sub-agent-tab">
    <SubAgentWindow :session-id="props.tab.subAgentSessionId ?? ''" />
  </div>
</template>

<style scoped>
.sub-agent-tab {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
</style>
