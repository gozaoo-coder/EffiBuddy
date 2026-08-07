<script setup lang="ts">
/**
 * CompressionStageBar —— 压缩阶段进度条
 *
 * 顶部阶段徽标(active/done/error 三态,旋转/对勾/叉号) + 阶段标签/说明 +
 * 耗时 + 进度条。
 */
import { inject } from 'vue'
import { Icon } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const { compressStage, compressStageMsg, compressElapsedMs, compressProgress, compressStageLabel } =
  store.compression
</script>

<template>
  <div class="compress-header">
    <div class="compress-stage-row">
      <div
        class="compress-stage-badge"
        :class="{
          'stage-active':
            compressStage !== 'idle' && compressStage !== 'done' && compressStage !== 'error',
          'stage-done': compressStage === 'done',
          'stage-error': compressStage === 'error',
        }"
      >
        <Icon v-if="compressStage === 'done'" name="check" :size="14" />
        <Icon v-else-if="compressStage === 'error'" name="close" :size="14" />
        <Icon v-else-if="compressStage !== 'idle'" name="loader" :size="14" />
        <Icon v-else name="merge" :size="14" />
      </div>
      <div class="compress-stage-text">
        <div class="compress-stage-label">{{ compressStageLabel }}</div>
        <div v-if="compressStageMsg" class="compress-stage-msg">
          {{ compressStageMsg }}
        </div>
      </div>
      <div v-if="compressStage === 'done' && compressElapsedMs > 0" class="compress-elapsed">
        {{ (compressElapsedMs / 1000).toFixed(1) }}s
      </div>
    </div>
    <!-- 进度条:仅在活跃阶段显示 -->
    <div
      v-if="compressStage !== 'idle' && compressStage !== 'error'"
      class="compress-progress-bar"
    >
      <div
        class="compress-progress-fill"
        :class="{ 'is-done': compressStage === 'done' }"
        :style="{ width: compressProgress * 100 + '%' }"
      />
    </div>
  </div>
</template>

<style scoped>
.compress-header {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.compress-stage-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.compress-stage-badge {
  flex: 0 0 auto;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--card-2);
  color: var(--muted);
  border: 1px solid var(--border);
    transition: background 0.2s ease, color 0.2s ease, border-color 0.2s ease;
}

.compress-stage-badge.stage-active {
  background: var(--accent, #4a7eff);
  color: #fff;
  border-color: var(--accent, #4a7eff);
  animation: compress-spin 1s linear infinite;
}

.compress-stage-badge.stage-done {
  background: var(--success, #10a37f);
  color: #fff;
  border-color: var(--success, #10a37f);
}

.compress-stage-badge.stage-error {
  background: var(--danger, #ef4444);
  color: #fff;
  border-color: var(--danger, #ef4444);
}

@keyframes compress-spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.compress-stage-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.compress-stage-label {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}

.compress-stage-msg {
  font-size: 12px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.compress-elapsed {
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.compress-progress-bar {
  height: 4px;
  border-radius: var(--radius-full);
  background: var(--bg-2);
  overflow: hidden;
}

.compress-progress-fill {
  height: 100%;
  border-radius: var(--radius-full);
  background: var(--primary);
  transition: width 0.3s ease;
}

.compress-progress-fill.is-done {
  background: var(--success);
}
</style>
