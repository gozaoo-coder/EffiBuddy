<script setup lang="ts">
/**
 * CompressionSettingsPanel —— 自动压缩设置面板（原子组件）
 *
 * 让"自动压缩"真正可操作：阈值滑块 + 自动压缩/工具调用/逐句对话开关。
 * 纯交互组件：编辑中的设置通过 `change` 事件上抛，持久化由父组件负责
 * （调用后端 update_compression_settings，不重建 agent）。
 */
import { reactive, watch } from 'vue'
import { Icon, Switch, Slider } from '../basic'
import type { CompressionSettings } from '../../types'

const props = withDefaults(
  defineProps<{
    /** 当前压缩设置 */
    settings: CompressionSettings
    /** 保存中（父组件调用后端期间置 true） */
    saving?: boolean
  }>(),
  {
    saving: false,
  },
)

const emit = defineEmits<{
  (e: 'change', v: CompressionSettings): void
}>()

// 本地草稿：父组件 settings 变化时同步
const draft = reactive<CompressionSettings>({ ...props.settings })

watch(
  () => props.settings,
  (s) => {
    draft.threshold_percent = s.threshold_percent
    draft.auto_compress = s.auto_compress
    draft.compress_tool_calls = s.compress_tool_calls
    draft.compress_sentences = s.compress_sentences
  },
  { deep: true },
)

function update(partial: Partial<CompressionSettings>) {
  Object.assign(draft, partial)
  emit('change', { ...draft })
}
</script>

<template>
  <div class="csp">
    <div class="csp-head">
      <Icon name="settings" :size="15" />
      <span>自动压缩设置</span>
      <span v-if="saving" class="csp-saving">
        <Icon name="loader" :size="12" />
        保存中…
      </span>
    </div>

    <div class="csp-row">
      <div class="csp-row-text">
        <div class="csp-row-title">自动压缩</div>
        <div class="csp-row-desc">上下文达到阈值时，回复完成后自动压缩历史</div>
      </div>
      <Switch
        :model-value="draft.auto_compress"
        :disabled="saving"
        size="sm"
        @change="(v: boolean) => update({ auto_compress: v })"
      />
    </div>

    <div class="csp-row csp-row-slider">
      <div class="csp-row-text">
        <div class="csp-row-title">
          触发阈值
          <span class="csp-threshold-val">{{ draft.threshold_percent }}%</span>
        </div>
        <div class="csp-row-desc">上下文使用量达到该比例时触发自动压缩</div>
      </div>
      <Slider
        :model-value="draft.threshold_percent"
        :min="10"
        :max="100"
        :step="5"
        :disabled="saving || !draft.auto_compress"
        size="sm"
        @update:model-value="(v: number) => update({ threshold_percent: v })"
      />
    </div>

    <div class="csp-row">
      <div class="csp-row-text">
        <div class="csp-row-title">压缩工具调用</div>
        <div class="csp-row-desc">对工具调用 / 工具返回做 Keep/Hide/Replace 精简</div>
      </div>
      <Switch
        :model-value="draft.compress_tool_calls"
        :disabled="saving"
        size="sm"
        @change="(v: boolean) => update({ compress_tool_calls: v })"
      />
    </div>

    <div class="csp-row">
      <div class="csp-row-text">
        <div class="csp-row-title">逐句对话压缩</div>
        <div class="csp-row-desc">按消息粒度精简长句，保留语义要点</div>
      </div>
      <Switch
        :model-value="draft.compress_sentences"
        :disabled="saving"
        size="sm"
        @change="(v: boolean) => update({ compress_sentences: v })"
      />
    </div>
  </div>
</template>

<style scoped>
.csp {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 12px 14px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.csp-head {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.csp-saving {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  font-weight: 400;
  color: var(--muted);
  margin-left: auto;
}

.csp-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.csp-row-slider {
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
}

.csp-row-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.csp-row-title {
  font-size: 13px;
  color: var(--text);
  font-weight: 500;
}

.csp-threshold-val {
  font-weight: 700;
  color: var(--primary);
  font-variant-numeric: tabular-nums;
}

.csp-row-desc {
  font-size: 11px;
  color: var(--muted);
  line-height: 1.5;
}
</style>
