<script setup lang="ts">
import { useSnackbar } from '../../composables/useFeedback'

const { state, dismiss } = useSnackbar()

function onAction(id: number, onClick?: () => void) {
  onClick?.()
  dismiss(id)
}
</script>

<template>
  <Teleport to="body">
    <div class="snackbar-host">
      <TransitionGroup name="snackbar">
        <div
          v-for="s in state.items"
          :key="s.id"
          class="snackbar"
          :class="{ 'snackbar--persistent': s.mode === 'persistent' }"
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
      </TransitionGroup>
    </div>
  </Teleport>
</template>
