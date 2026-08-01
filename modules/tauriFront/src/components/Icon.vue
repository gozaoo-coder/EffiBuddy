<script setup lang="ts">
/**
 * Icon 图标组件（薄壳）
 *
 * 底层由 Hugeicons 驱动：semantic name → iconMap → HugeIcon 渲染 SVG。
 * - 图标数据：@hugeicons/core-free-icons（5400+ Stroke Rounded，MIT）
 * - 渲染器：./icons/HugeIcon.ts（复刻官方 @hugeicons/vue 逻辑，规避其类型声明缺失）
 * - 映射表：./icons/iconMap.ts（80+ 语义别名，覆盖项目全部用例）
 *
 * API 与旧实现完全兼容：
 *   <Icon name="menu" :size="18" />
 *   <Icon name="search" :size="18" fallback="?" />
 *
 * 主题适配：SVG 的 fill/stroke 继承 currentColor，由父元素 color 控制。
 * 未命中 name 时渲染 fallback 字符（不输出 svg）。
 */
import { computed } from 'vue'
import { HugeIcon } from './icons/HugeIcon'
import { resolveIcon, type IconData } from './icons/iconMap'

const props = withDefaults(
  defineProps<{
    /** 语义名（如 menu / delete / search / close / check / chevron-down） */
    name: string
    /** 尺寸 px，默认 18 */
    size?: number | string
    /** 当 name 未命中时显示的回退字符 */
    fallback?: string
  }>(),
  {
    size: 18,
    fallback: '',
  },
)

// 命中的图标数据；未命中返回 undefined，触发 fallback 字符渲染
const icon = computed<IconData | undefined>(() => resolveIcon(props.name))

// 解析尺寸：仅接受正数；非法值回退到默认 18
const sizeNum = computed(() => {
  const raw = typeof props.size === 'string' ? parseInt(props.size, 10) : props.size
  return !isNaN(raw) && raw > 0 ? raw : 18
})

const sizeStyle = computed(() => ({
  width: `${sizeNum.value}px`,
  height: `${sizeNum.value}px`,
}))
</script>

<template>
  <span class="app-icon" :style="sizeStyle">
    <HugeIcon v-if="icon" :icon="icon" :size="sizeNum" />
    <template v-else>{{ fallback }}</template>
  </span>
</template>

<style scoped>
.app-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  vertical-align: middle;
  line-height: 0;
}

/* HugeIcon 输出的 svg 自带 width/height，这里仅清除行内空白 */
.app-icon :deep(svg) {
  display: block;
}
</style>
