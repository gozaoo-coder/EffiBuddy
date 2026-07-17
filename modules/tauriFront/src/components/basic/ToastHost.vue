<script setup lang="ts">
import { useToast } from '../../composables/useFeedback'

const { state, dismiss } = useToast()

const typeIcon: Record<string, string> = {
  info: 'ℹ',
  success: '✓',
  warn: '⚠',
  error: '✕',
}
</script>

<template>
  <Teleport to="body">
    <!-- 顶部 toast 容器 -->
    <div class="toast-host toast-host--top">
      <TransitionGroup name="toast-top">
        <div
          v-for="t in state.items.filter((i) => i.position === 'top')"
          :key="t.id"
          class="toast"
          :class="`toast--${t.type}`"
          @click="dismiss(t.id)"
        >
          <span class="toast-icon">{{ typeIcon[t.type] }}</span>
          <span class="toast-content">{{ t.content }}</span>
        </div>
      </TransitionGroup>
    </div>

    <!-- 底部 toast 容器 -->
    <div class="toast-host toast-host--bottom">
      <TransitionGroup name="toast-bottom">
        <div
          v-for="t in state.items.filter((i) => i.position === 'bottom')"
          :key="t.id"
          class="toast"
          :class="`toast--${t.type}`"
          @click="dismiss(t.id)"
        >
          <span class="toast-icon">{{ typeIcon[t.type] }}</span>
          <span class="toast-content">{{ t.content }}</span>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>
