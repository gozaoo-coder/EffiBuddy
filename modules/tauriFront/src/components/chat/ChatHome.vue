<script setup lang="ts">
/**
 * ChatHome —— 空状态首页
 *
 * Kimi 风格中央品牌区 + 快捷胶囊。
 */
import { inject } from 'vue'
import { Icon, Chips } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const { quickActions, applyQuickAction, activeModelInfo } = store.core
</script>

<template>
  <div class="home-empty">
    <div class="home-brand">
      <div class="home-logo">
        <span class="home-logo-text">Effi</span>
        <span class="home-logo-icon"><Icon name="robot" :size="48" /></span>
        <span class="home-logo-text">Buddy</span>
      </div>
      <div class="home-subtitle">
        {{ activeModelInfo?.name || 'EffiBuddy' }}
      </div>
      <div class="home-subtitle secondary">为你进化</div>
    </div>

    <div class="home-actions">
      <Chips
        v-for="action in quickActions"
        :key="action.label"
        :label="action.label"
        size="md"
        @click="applyQuickAction(action.label)"
      >
        <template #icon><Icon :name="action.icon" :size="16" /></template>
      </Chips>
    </div>
  </div>
</template>

<style scoped>
/* Kimi 风格空状态首页 */
.home-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 28px;
  padding-bottom: 10vh;
}

.home-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.home-logo {
  display: flex;
  align-items: center;
  gap: 8px;
}

.home-logo-text {
  font-size: 32px;
  font-weight: 800;
  letter-spacing: -0.5px;
  color: var(--text);
  background: linear-gradient(135deg, var(--primary), color-mix(in srgb, var(--primary) 60%, var(--accent)));
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

.home-logo-icon {
  display: inline-flex;
  color: var(--primary);
}

.home-subtitle {
  font-size: 14px;
  color: var(--text);
  font-weight: 500;
}

.home-subtitle.secondary {
  font-size: 13px;
  color: var(--muted);
  font-weight: 400;
}

.home-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  justify-content: center;
}
</style>
