<script setup lang="ts">
/**
 * CompressionGauge —— 上下文用量仪表盘（原子组件）
 *
 * 用横向比例条直观展示"当前上下文用量"在"窗口"中的位置：
 *  - 已用填充：按用量占比着色（<70% 绿 / 70-90% 橙 / >90% 红）
 *  - 阈值线：自动压缩触发阈值处的竖线 + 标签
 *  - 压缩后刻度：有压缩后预估时，在对应位置画虚线刻度
 *  - 底部：used / max tokens 数值 + 阈值百分比 + 压缩后预估
 *
 * 纯展示组件：所有数据通过 props 传入，不依赖任何 store。
 */
import { computed } from 'vue'
import { Icon } from '../basic'

const props = withDefaults(
  defineProps<{
    /** 当前上下文用量（tokens） */
    usedTokens: number
    /** 上下文窗口（tokens） */
    maxTokens: number
    /** 自动压缩触发阈值（百分比 0-100） */
    thresholdPercent?: number
    /** 压缩后预估用量（tokens）；null = 无压缩后数据 */
    afterTokens?: number | null
    /** 标题，默认「上下文使用」 */
    label?: string
  }>(),
  {
    thresholdPercent: 80,
    afterTokens: null,
    label: '上下文使用',
  },
)

const usedPct = computed(() => {
  if (props.maxTokens <= 0) return 0
  return Math.min(100, Math.max(0, (props.usedTokens / props.maxTokens) * 100))
})

const afterPct = computed(() => {
  if (props.afterTokens == null || props.maxTokens <= 0) return null
  return Math.min(100, Math.max(0, (props.afterTokens / props.maxTokens) * 100))
})

const tone = computed(() => {
  const r = props.maxTokens > 0 ? props.usedTokens / props.maxTokens : 0
  if (r > 0.9) return 'danger'
  if (r >= 0.7) return 'warn'
  return 'success'
})

const thresholdPct = computed(() => Math.min(100, Math.max(0, props.thresholdPercent)))

const fmt = (n: number) => Math.round(n).toLocaleString('zh-CN')
</script>

<template>
  <div class="cg">
    <div class="cg-head">
      <span class="cg-title">
        <Icon name="view" :size="14" />
        {{ label }}
      </span>
      <span class="cg-pct" :class="`is-${tone}`">{{ Math.round(usedPct) }}%</span>
    </div>

    <div class="cg-track" :class="`is-${tone}`">
      <div class="cg-fill" :style="{ width: usedPct + '%' }" />
      <!-- 自动压缩阈值线 -->
      <div class="cg-marker cg-threshold" :style="{ left: thresholdPct + '%' }">
        <span class="cg-marker-line" />
      </div>
      <!-- 压缩后预估刻度 -->
      <div v-if="afterPct != null" class="cg-marker cg-after" :style="{ left: afterPct + '%' }">
        <span class="cg-marker-line is-dashed" />
      </div>
    </div>

    <div class="cg-labels">
      <span class="cg-used">{{ fmt(usedTokens) }} / {{ fmt(maxTokens) }} tokens</span>
      <span class="cg-threshold-label">
        阈值 @ {{ Math.round(thresholdPct) }}%
      </span>
    </div>

    <div v-if="afterPct != null" class="cg-after-label">
      压缩后约 <b>{{ fmt(afterTokens ?? 0) }}</b> tokens（{{ Math.round(afterPct ?? 0) }}%）
    </div>
  </div>
</template>

<style scoped>
.cg {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.cg-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.cg-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.cg-pct {
  font-size: 15px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}

.cg-pct.is-success { color: var(--success); }
.cg-pct.is-warn { color: var(--warn); }
.cg-pct.is-danger { color: var(--danger); }

/* 比例条 */
.cg-track {
  position: relative;
  height: 10px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--muted) 22%, transparent);
  overflow: visible;
}

.cg-fill {
  height: 100%;
  border-radius: 999px;
  transition: width 0.4s ease;
}

.cg-track.is-success .cg-fill {
  background: linear-gradient(90deg, color-mix(in srgb, var(--success) 60%, transparent), var(--success));
}
.cg-track.is-warn .cg-fill {
  background: linear-gradient(90deg, color-mix(in srgb, var(--warn) 60%, transparent), var(--warn));
}
.cg-track.is-danger .cg-fill {
  background: linear-gradient(90deg, color-mix(in srgb, var(--danger) 60%, transparent), var(--danger));
}

/* 标记（阈值线 / 压缩后刻度） */
.cg-marker {
  position: absolute;
  top: -3px;
  bottom: -3px;
  width: 0;
  display: flex;
  align-items: center;
  pointer-events: none;
}

.cg-marker-line {
  width: 2px;
  height: 100%;
  border-radius: 2px;
  background: var(--muted);
  opacity: 0.85;
}

.cg-marker-line.is-dashed {
  background: repeating-linear-gradient(
    to bottom,
    var(--success) 0 3px,
    transparent 3px 6px
  );
  opacity: 1;
}

.cg-threshold .cg-marker-line {
  background: var(--warn);
}

/* 底部信息行 */
.cg-labels {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}

.cg-used {
  font-weight: 600;
  color: var(--text);
}

.cg-threshold-label {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.cg-after-label {
  font-size: 12px;
  color: var(--success);
  font-variant-numeric: tabular-nums;
}

.cg-after-label b {
  font-weight: 700;
}
</style>
