<script setup lang="ts">
import { useTheme } from '../composables/useTheme'
import type { ThemeMode } from '../types'

const { themeMode, setTheme } = useTheme()

const options: { value: ThemeMode; label: string; icon: string }[] = [
  { value: 'light', label: '亮色', icon: '☀' },
  { value: 'dark', label: '暗色', icon: '☾' },
  { value: 'system', label: '系统', icon: '⌂' },
]
</script>

<template>
  <div class="theme-switcher" role="radiogroup" aria-label="主题模式">
    <button
      v-for="opt in options"
      :key="opt.value"
      class="ts-btn"
      :class="{ active: themeMode === opt.value }"
      :aria-checked="themeMode === opt.value"
      role="radio"
      :title="opt.label"
      @click="setTheme(opt.value)"
    >
      <span class="ts-icon">{{ opt.icon }}</span>
      <span class="ts-label">{{ opt.label }}</span>
    </button>
  </div>
</template>
