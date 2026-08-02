<script setup lang="ts">
/**
 * ModelSettingsContent 模型设置主内容区容器
 *
 * 根据当前选中的二级子项（view）切换显示对应面板：
 * - 'providers'：AI 服务商面板（编辑预设服务商 + 按能力类型填入具体模型）
 * - 'roles'：服务模型面板（配置各使用场景的默认模型）
 * - ''：默认介绍页（未选中任何子项时展示）
 *
 * 容器职责：
 * - 路由 view → 对应子面板
 * - 提供子面板间切换的淡入动画
 * - 透传 saved 事件给父级（App.vue）
 *
 * 不持有业务状态：子面板各自管理 config / presets 等数据，
 * 容器仅做视图路由，符合"减少上帝文件"原则。
 */
import { computed } from 'vue'
import { useAnimeTransition } from '../../composables/useAnimeTransition'
import { Icon } from '../basic'
import ProviderPanel from './ProviderPanel.vue'
import ServiceRolesPanel from './ServiceRolesPanel.vue'
import type { ModelSettingsView } from './ModelSettingsRail.vue'

const props = defineProps<{
  /** 当前选中的二级子项（'' 表示未选中，显示默认介绍页） */
  view: ModelSettingsView | ''
}>()

const emit = defineEmits<{
  (e: 'saved'): void
}>()

const currentPanel = computed<'providers' | 'roles' | 'intro'>(() => {
  if (props.view === 'providers') return 'providers'
  if (props.view === 'roles') return 'roles'
  return 'intro'
})

// 子面板切换淡入动画
const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [10, 0],
    duration: 260,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateY: [0, -6],
    duration: 180,
    ease: 'inOut(2)',
  },
})

function onChildSaved() {
  emit('saved')
}
</script>

<template>
  <section class="ms-content">
    <Transition :css="false" @enter="onEnter" @leave="onLeave" mode="out-in">
      <!-- 默认介绍页 -->
      <div v-if="currentPanel === 'intro'" key="intro" class="ms-intro">
        <div class="ms-intro-card">
          <span class="ms-intro-glyph">
            <Icon name="robot" :size="32" />
          </span>
          <h2 class="ms-intro-title">模型设置</h2>
          <p class="ms-intro-desc">
            在左侧选择子项开始配置：
          </p>
          <ul class="ms-intro-list">
            <li>
              <Icon name="globe" :size="14" />
              <strong>AI 服务商</strong>：编辑预设服务商，按能力类型（对话/生图/生视频/音频转文字）填入具体模型
            </li>
            <li>
              <Icon name="robot" :size="14" />
              <strong>服务模型</strong>：配置各使用场景的默认模型（聊天/命名/压缩/语音/生图）
            </li>
          </ul>
        </div>
      </div>

      <!-- AI 服务商面板 -->
      <ProviderPanel
        v-else-if="currentPanel === 'providers'"
        key="providers"
        @saved="onChildSaved"
      />

      <!-- 服务模型面板 -->
      <ServiceRolesPanel
        v-else
        key="roles"
        @saved="onChildSaved"
      />
    </Transition>
  </section>
</template>

<style scoped>
.ms-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  overflow: hidden;
}

/* 默认介绍页 */
.ms-intro {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 32px;
  overflow-y: auto;
}

.ms-intro-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 40px 36px;
  max-width: 480px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-sm);
  text-align: center;
}

.ms-intro-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.14), rgba(108, 92, 231, 0.14));
  color: var(--primary);
  margin-bottom: 4px;
}

.ms-intro-title {
  margin: 0;
  font-size: 20px;
  font-weight: 700;
  color: var(--text);
  letter-spacing: 0.3px;
}

.ms-intro-desc {
  margin: 0 0 8px;
  font-size: var(--fs-sm);
  color: var(--muted);
}

.ms-intro-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  text-align: left;
  width: 100%;
}

.ms-intro-list li {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 10px 12px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
  color: var(--text);
  line-height: 1.5;
}

.ms-intro-list li :deep(svg) {
  flex-shrink: 0;
  margin-top: 2px;
  color: var(--primary);
}

.ms-intro-list strong {
  font-weight: 600;
  color: var(--text);
  margin-right: 4px;
}
</style>
