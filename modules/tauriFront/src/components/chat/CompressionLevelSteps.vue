<script setup lang="ts">
/**
 * CompressionLevelSteps —— 压缩效果指标卡（原子组件，重构版）
 *
 * 用真实 token 指标（后端取自 API responses 的 usage）替代原来的"压缩态阶梯"：
 *  - 已压缩 n tokens：base - current 的累计压缩量
 *  - 比未压缩前节省 n%：saved / base
 *  - 上一轮压缩大小 n%：current / base（当前有效历史占未压缩的比例）
 * 顶部保留压缩等级徽标（L1/L2/…）与"基于上次压缩态进一步精简"的说明。
 *
 * 纯展示组件，所有数据通过 props 传入。
 */
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
      /** 当前压缩等级（0 = 未压缩，N = 已压缩 N 次，无上限） */
    level: number
    /** 完全未压缩历史段的真实 token 数（0 = 旧数据未回填） */
    baseTokens?: number
    /** 压缩后的当前有效历史真实 token 数 */
    currentTokens?: number
  }>(),
  {
    baseTokens: 0,
    currentTokens: 0,
  },
)

const hasTokens = computed(() => props.baseTokens > 0 && props.currentTokens > 0)
const savedTokens = computed(() => Math.max(0, props.baseTokens - props.currentTokens))
const savedPercent = computed(() =>
  props.baseTokens > 0 ? Math.round((savedTokens.value / props.baseTokens) * 100) : 0,
)
const remainPercent = computed(() =>
  props.baseTokens > 0 ? Math.round((props.currentTokens / props.baseTokens) * 100) : 0,
)

/** 千分位格式化 */
function fmt(n: number): string {
  return n.toLocaleString('zh-CN')
}
</script>

<template>
  <div class="cls">
    <!-- 顶部：等级徽标 + 递进说明 -->
      <div class="cls-head">
        <span class="cls-badge">
          L{{ level }}
        </span>
        <span class="cls-head-text">
          <template v-if="level > 0">每次压缩基于上次压缩态进一步精简（无上限）</template>
          <template v-else>尚未压缩 · 点击开始压缩释放上下文空间</template>
        </span>
      </div>

    <!-- 真实 token 指标（旧数据未回填时降级为说明） -->
    <template v-if="hasTokens">
      <div class="cls-grid">
        <div class="cls-cell is-saved">
          <div class="cls-cell-label">已压缩</div>
          <div class="cls-cell-value">{{ fmt(savedTokens) }}</div>
          <div class="cls-cell-unit">tokens</div>
        </div>
        <div class="cls-cell is-save">
          <div class="cls-cell-label">比未压缩前节省</div>
          <div class="cls-cell-value">{{ savedPercent }}%</div>
          <div class="cls-cell-unit">节省 {{ fmt(savedTokens) }} tokens</div>
        </div>
        <div class="cls-cell is-remain">
          <div class="cls-cell-label">上一轮压缩大小</div>
          <div class="cls-cell-value">{{ remainPercent }}%</div>
          <div class="cls-cell-unit">当前 / 未压缩</div>
        </div>
      </div>
      <div class="cls-foot">
        已压缩 {{ fmt(savedTokens) }} tokens · 比完全未压缩前节省 {{ savedPercent }}% · 当前为未压缩的
        {{ remainPercent }}%
      </div>
    </template>
    <div v-else class="cls-empty">
      完成压缩后将以真实 token 展示节省量（取自 API responses 的 usage 字段）。
    </div>
  </div>
</template>

<style scoped>
.cls {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.cls-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.cls-badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  font-size: 11px;
  font-weight: 700;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  white-space: nowrap;
}


.cls-head-text {
  font-size: 11px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 三格指标 */
.cls-grid {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 8px;
}

.cls-cell {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 10px 6px;
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--muted) 8%, transparent);
  min-width: 0;
}

.cls-cell-label {
  font-size: 11px;
  color: var(--muted);
  white-space: nowrap;
}

.cls-cell-value {
  font-size: 20px;
  font-weight: 800;
  font-variant-numeric: tabular-nums;
  line-height: 1.2;
}

.cls-cell-unit {
  font-size: 10px;
  color: var(--muted);
  white-space: nowrap;
}

.cls-cell.is-saved .cls-cell-value {
  color: var(--primary);
}

.cls-cell.is-save .cls-cell-value {
  color: var(--success);
}

.cls-cell.is-remain .cls-cell-value {
  color: var(--text);
}

.cls-foot {
  font-size: 11px;
  color: var(--muted);
  text-align: center;
}

.cls-empty {
  font-size: 12px;
  color: var(--muted);
  text-align: center;
  padding: 6px 0;
}
</style>
