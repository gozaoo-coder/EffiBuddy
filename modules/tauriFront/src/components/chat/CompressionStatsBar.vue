<script setup lang="ts">
/**
 * CompressionStatsBar —— 压缩决策统计占比条（原子组件）
 *
 * 把 keep / hide / replace 的纯数字统计升级为"堆叠占比条 + 图例"，
 * 一眼看出三类决策各占多少、涉及多少消息。
 *
 * 纯展示组件，所有数据通过 props 传入。
 */
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    /** 保持决策条数 */
    keep: number
    /** 隐藏决策条数 */
    hide: number
    /** 替换决策条数 */
    replace: number
    /** 涉及消息总数（去重后） */
    totalIds: number
  }>(),
  {
    keep: 0,
    hide: 0,
    replace: 0,
    totalIds: 0,
  },
)

const total = computed(() => props.keep + props.hide + props.replace)

const segments = computed(() => {
  if (total.value === 0) return []
  const mk = (key: 'keep' | 'hide' | 'replace', label: string, cls: string) => ({
    key,
    label,
    cls,
    count: props[key],
    pct: Math.round((props[key] / total.value) * 100),
    width: `${(props[key] / total.value) * 100}%`,
  })
  return [mk('keep', '保持', 'is-keep'), mk('hide', '隐藏', 'is-hide'), mk('replace', '替换', 'is-replace')]
})
</script>

<template>
  <div class="csb">
    <div class="csb-bar">
      <template v-if="total > 0">
        <div
          v-for="seg in segments"
          :key="seg.key"
          class="csb-seg"
          :class="seg.cls"
          :style="{ width: seg.width }"
          :title="`${seg.label} ${seg.count}（${seg.pct}%）`"
        />
      </template>
      <div v-else class="csb-empty" />
    </div>

    <div class="csb-legend">
      <div v-for="seg in segments" :key="seg.key" class="csb-legend-item">
        <span class="csb-dot" :class="seg.cls" />
        <span class="csb-name">{{ seg.label }}</span>
        <span class="csb-count">{{ seg.count }}</span>
        <span class="csb-pct">{{ seg.pct }}%</span>
      </div>
      <div class="csb-legend-item">
        <span class="csb-name">涉及消息</span>
        <span class="csb-count is-total">{{ totalIds }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.csb {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

/* 堆叠占比条 */
.csb-bar {
  display: flex;
  height: 10px;
  border-radius: 999px;
  overflow: hidden;
  background: color-mix(in srgb, var(--muted) 18%, transparent);
}

.csb-seg {
  height: 100%;
  transition: width 0.4s ease;
}

.csb-seg.is-keep { background: var(--success); }
.csb-seg.is-hide { background: color-mix(in srgb, var(--muted) 65%, var(--border)); }
.csb-seg.is-replace { background: var(--warn); }

.csb-empty {
  width: 100%;
  height: 100%;
}

/* 图例 */
.csb-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 14px;
}

.csb-legend-item {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  color: var(--text);
}

.csb-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.csb-dot.is-keep { background: var(--success); }
.csb-dot.is-hide { background: color-mix(in srgb, var(--muted) 65%, var(--border)); }
.csb-dot.is-replace { background: var(--warn); }

.csb-name {
  color: var(--muted);
}

.csb-count {
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.csb-count.is-total {
  color: var(--text);
  font-weight: 700;
}

.csb-pct {
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  min-width: 34px;
  text-align: right;
}
</style>
