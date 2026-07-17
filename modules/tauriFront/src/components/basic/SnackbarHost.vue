<script setup lang="ts">
import { ref, watch, nextTick } from 'vue'
import { useSnackbar } from '../../composables/useFeedback'
import { useLayout } from '../../composables/useLayout'

const { state, dismiss } = useSnackbar()

function onAction(id: number, onClick?: () => void) {
  onClick?.()
  dismiss(id)
}

// snackbar 容器引用
const rootEl = ref<HTMLElement | null>(null)

// anime.js v4 Layout：驱动 snackbar 列表的进入/离开动画
// enterFrom/leaveTo 必须使用完整 transform 字符串
const { record, animate } = useLayout(rootEl, {
  children: '.snackbar',
  duration: 280,
  ease: 'outQuad',
  enterFrom: { opacity: 0, transform: 'translateY(40px)' },
  leaveTo: { opacity: 0, transform: 'translateY(20px) scale(.95)' },
})

/**
 * 监听 snackbar 列表变化（新增 / hiding 标志切换），驱动 anime.js Layout 动画。
 * 详见 ToastHost.vue 中同名逻辑的说明。
 */
watch(
  () => state.items.map((s) => `${s.id}:${s.hiding ? 'h' : 'v'}`).join('|'),
  async () => {
    record()
    await nextTick()
    await animate()
  },
)
</script>

<template>
  <Teleport to="body">
    <div ref="rootEl" class="snackbar-host">
      <div
        v-for="s in state.items"
        :key="s.id"
        class="snackbar"
        :class="[{ 'snackbar--persistent': s.mode === 'persistent' }, { 'is-hidden': s.hiding }]"
      >
        <span class="snackbar-content">{{ s.content }}</span>
        <div class="snackbar-actions">
          <button
            v-if="s.action"
            class="snackbar-action"
            @click="onAction(s.id, s.action.onClick)"
          >{{ s.action.text }}</button>
          <button
            v-if="s.mode === 'persistent'"
            class="snackbar-close"
            aria-label="关闭"
            @click="dismiss(s.id)"
          >×</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
