<script setup lang="ts">
import { ref, computed, watch, nextTick } from 'vue'
import { useToast } from '../../composables/useFeedback'
import { useLayout } from '../../composables/useLayout'
import Icon from '../Icon.vue'

const { state, dismiss } = useToast()

const typeIcon: Record<string, string> = {
  info: 'info',
  success: 'check-builtin',
  warn: 'warning',
  error: 'close',
}

// 顶部 / 底部 toast 容器引用
const topRootEl = ref<HTMLElement | null>(null)
const bottomRootEl = ref<HTMLElement | null>(null)

const topItems = computed(() => state.items.filter((i) => i.position === 'top'))
const bottomItems = computed(() => state.items.filter((i) => i.position === 'bottom'))

// anime.js v4 Layout：驱动 toast 列表的进入/离开动画
// enterFrom/leaveTo 必须使用完整 transform 字符串（Layout 模块不支持 transform 简写）
const { record: recordTop, animate: animateTop } = useLayout(topRootEl, {
  children: '.toast',
  duration: 280,
  ease: 'outQuad',
  enterFrom: { opacity: 0, transform: 'translateY(-20px) scale(.9)' },
  leaveTo: { opacity: 0, transform: 'translateY(-10px) scale(.95)' },
})

const { record: recordBottom, animate: animateBottom } = useLayout(bottomRootEl, {
  children: '.toast',
  duration: 280,
  ease: 'outQuad',
  enterFrom: { opacity: 0, transform: 'translateY(20px) scale(.9)' },
  leaveTo: { opacity: 0, transform: 'translateY(10px) scale(.95)' },
})

/**
 * 监听列表项变化（新增 / hiding 标志切换），驱动 anime.js Layout 动画。
 *
 * 原理：watch 默认 flush:'pre'，在 Vue 渲染前触发。
 * 1. record() 快照当前 DOM（变更前的旧状态）
 * 2. await nextTick() —— 让 Vue 完成 DOM patch（新增元素 / 加上 is-hidden 触发 display:none）
 * 3. animate() —— anime.js Layout 检测到 enterFrom / leaveTo 差异并播放过渡
 *
 * 对于离开：dismiss 在 useFeedback 中仅设置 hiding=true（不立即 splice），
 * 元素因 is-hidden 变成 display:none，anime.js Layout 自动以 leaveTo 动画收尾，
 * 动画时长后再由 setTimeout 真正从数据源移除。
 */
watch(
  () => topItems.value.map((i) => `${i.id}:${i.hiding ? 'h' : 'v'}`).join('|'),
  async () => {
    recordTop()
    await nextTick()
    await animateTop()
  },
)

watch(
  () => bottomItems.value.map((i) => `${i.id}:${i.hiding ? 'h' : 'v'}`).join('|'),
  async () => {
    recordBottom()
    await nextTick()
    await animateBottom()
  },
)
</script>

<template>
  <Teleport to="body">
    <!-- 顶部 toast 容器 -->
    <div ref="topRootEl" class="toast-host toast-host--top">
      <div
        v-for="t in topItems"
        :key="t.id"
        class="toast"
        :class="[`toast--${t.type}`, { 'is-hidden': t.hiding }]"
        @click="dismiss(t.id)"
      >
        <span class="toast-icon"><Icon :name="typeIcon[t.type]" :size="18" /></span>
        <span class="toast-content">{{ t.content }}</span>
      </div>
    </div>

    <!-- 底部 toast 容器 -->
    <div ref="bottomRootEl" class="toast-host toast-host--bottom">
      <div
        v-for="t in bottomItems"
        :key="t.id"
        class="toast"
        :class="[`toast--${t.type}`, { 'is-hidden': t.hiding }]"
        @click="dismiss(t.id)"
      >
        <span class="toast-icon"><Icon :name="typeIcon[t.type]" :size="18" /></span>
        <span class="toast-content">{{ t.content }}</span>
      </div>
    </div>
  </Teleport>
</template>
