<script setup lang="ts">
/**
 * 主题切换器：基于 SegmentedButton 组件实现
 * 三档：亮色 ☀ / 暗色 ☾ / 系统 ⌂
 */
import { useTheme } from '../composables/useTheme'
import { SegmentedButton, type SegmentedOption } from './basic'
import type { ThemeMode } from '../types'

const { themeMode, setTheme } = useTheme()

const options: SegmentedOption[] = [
  { label: '亮色', value: 'light', icon: '☀' },
  { label: '暗色', value: 'dark', icon: '☾' },
  { label: '跟随系统', value: 'system', icon: '⌂' },
]

function onChange(_v: string | number, opt: SegmentedOption) {
  setTheme(opt.value as ThemeMode)
}
</script>

<template>
  <SegmentedButton
    :model-value="themeMode"
    :options="options"
    size="sm"
    @change="onChange"
  />
</template>
