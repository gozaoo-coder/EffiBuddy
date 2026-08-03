<script setup lang="ts">
/**
 * PluginPageTab —— 插件页签容器
 *
 * 根据 tab.pluginPageId 从 usePluginPages 页面注册表解析组件并渲染。
 * 未注册的页面 id 显示友好降级占位（提示安装对应插件）。
 */
import { computed } from 'vue'
import Icon from '../Icon.vue'
import { usePluginPages } from '../../composables/usePluginPages'
import type { TabItem } from '../../types'

defineOptions({ name: 'PluginPageTab' })

const props = defineProps<{
  tab: TabItem
}>()

const { resolvePageComponent } = usePluginPages()

const component = computed(() =>
  props.tab.pluginPageId ? resolvePageComponent(props.tab.pluginPageId) : null,
)
</script>

<template>
  <div class="plugin-page-tab">
    <!-- 已注册页面：渲染对应组件 -->
    <component :is="component" v-if="component" />

    <!-- 未注册页面：降级占位 -->
    <div v-else class="plugin-page-missing">
      <span class="missing-icon"><Icon name="puzzle" :size="44" /></span>
      <h3 class="missing-title">页面不可用</h3>
      <p class="missing-desc">
        未找到插件页面「{{ props.tab.pluginPageId }}」。该页面可能来自尚未安装的插件，
        或插件 manifest 已更新。请在「插件」面板中检查对应插件状态。
      </p>
    </div>
  </div>
</template>

<style scoped>
.plugin-page-tab {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

.plugin-page-missing {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 40px;
  text-align: center;
}

.missing-icon {
  display: inline-flex;
  color: var(--muted);
  opacity: 0.5;
}

.missing-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text);
}

.missing-desc {
  margin: 0;
  max-width: 400px;
  font-size: var(--fs-sm);
  line-height: 1.6;
  color: var(--muted);
}
</style>
