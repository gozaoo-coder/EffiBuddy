<script setup lang="ts">
/**
 * CompressionSheet —— 消息压缩浮窗（编排壳，重构版）
 *
 * 数据仪表盘化编排：
 *  - 上下文用量仪表盘（CompressionGauge：当前用量 / 阈值线 / 压缩后预估）
 *  - 压缩效果指标卡（CompressionLevelSteps：已压缩 n tokens / 节省 n% / 上一轮压缩大小 n%）
 *  - 决策统计占比条（CompressionStatsBar：keep/hide/replace 堆叠 + 图例）
 *  - 自动压缩设置面板（CompressionSettingsPanel，可展开，保存不重建 agent）
 *  - 阶段进度条 / 错误提示 / 流式输出 / 决策列表（保留）
 *  - 底部操作增强：再次压缩升级（done 且未达上限）/ 清除压缩状态（danger）
 *
 * 卡片级 UI 全部原子化为子组件，本文件只保留容器与编排逻辑。
 */
import { computed, inject, ref } from 'vue'
import MarkdownRender from 'markstream-vue'
import { Button, Icon, BindSheet } from '../basic'
import CompressionStageBar from './CompressionStageBar.vue'
import CompressionActionCard from './CompressionActionCard.vue'
import CompressionGauge from './CompressionGauge.vue'
import CompressionLevelSteps from './CompressionLevelSteps.vue'
import CompressionStatsBar from './CompressionStatsBar.vue'
import CompressionSettingsPanel from './CompressionSettingsPanel.vue'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const {
  compressionSheetOpen,
  compressStage,
  compressRawText,
  compressActions,
  compressError,
  compressExistingState,
  compressing,
  streamParsedActions,
  expandedActions,
  compressActionStats,
  compressSavedInfo,
  compressLevel,
  compressBaseTokens,
  compressCurrentTokens,
  compressionSettings,
  compressingSettings,
  toggleActionExpand,
  triggerCompress,
  closeCompressionSheet,
  clearCompression,
  loadCompressionSettings,
  saveCompressionSettings,
} = store.compression

const core = store.core

/** 最高压缩等级（与后端 MAX_COMPRESSION_LEVEL 对齐） */
const MAX_LEVEL = 3

// ---------- 上下文用量仪表盘数据 ----------
const ctxUsed = computed(() => core.contextUsedTokens.value)
const ctxMax = computed(() => core.contextMaxTokens.value)
const ctxThreshold = computed(() => compressionSettings.value?.threshold_percent ?? 80)
// 压缩后预估用量 = 当前用量 - 已节省 tokens（无节省数据时为 null）
const ctxAfter = computed(() => {
  const saved = compressSavedInfo.value?.savedTokens ?? 0
  if (saved <= 0) return null
  return Math.max(0, ctxUsed.value - saved)
})

// 当前是否存在压缩状态（底部"清除"按钮的依据）
const hasCompressionState = computed(
  () => !!compressExistingState.value || compressActions.value.length > 0,
)
const isMaxLevel = computed(() => compressLevel.value >= MAX_LEVEL)

// 统计展示：done 用本轮结果，idle+existing 用既有状态
const statKeep = computed(() =>
  compressStage.value === 'done'
    ? compressActionStats.value.keep
    : (compressExistingState.value?.actions.filter((a) => a.method === 'keep').length ?? 0),
)
const statHide = computed(() =>
  compressStage.value === 'done'
    ? compressActionStats.value.hide
    : (compressExistingState.value?.actions.filter((a) => a.method === 'hide').length ?? 0),
)
const statReplace = computed(() =>
  compressStage.value === 'done'
    ? compressActionStats.value.replace
    : (compressExistingState.value?.actions.filter((a) => a.method === 'replace').length ?? 0),
)
const statTotalIds = computed(() =>
  compressStage.value === 'done'
    ? compressActionStats.value.totalIds
    : (compressExistingState.value?.actions.reduce((s, a) => s + a.message_ids.length, 0) ?? 0),
)

// 设置面板展开/收起
const settingsOpen = ref(false)

// 设置变更 → 后端保存
async function onSettingsChange(s: Parameters<typeof saveCompressionSettings>[0]) {
  await saveCompressionSettings(s)
}

// 三种决策列表数据源:streaming(实时) / done(最终) / existing(历史)
const actionSources = [
  {
    key: 'stream' as const,
    visible: () => compressStage.value === 'streaming' && streamParsedActions.value.length > 0,
    title: () => `实时决策（${streamParsedActions.value.length} 条）`,
    hint: '· 已解析',
    actions: () => streamParsedActions.value,
    interactive: false,
  },
  {
    key: 'done' as const,
    visible: () => compressStage.value === 'done' && compressActions.value.length > 0,
    title: () => `压缩决策（第 ${compressLevel.value} 级 · ${compressActions.value.length} 条 · 点击展开查看原文）`,
    hint: '',
    actions: () => compressActions.value,
    interactive: true,
  },
  {
    key: 'existing' as const,
    visible: () =>
      compressStage.value === 'idle' &&
      !!compressExistingState.value &&
      compressExistingState.value.actions.length > 0,
    title: () => `上次压缩结果（第 ${compressLevel.value} 级）`,
    hint: '',
    actions: () => compressExistingState.value?.actions ?? [],
    interactive: true,
  },
] as const
</script>

<template>
  <BindSheet v-model:visible="compressionSheetOpen" title="消息压缩" side="bottom" :height="'85vh'">
    <div class="compress-sheet">
      <!-- 顶部阶段进度条 -->
      <CompressionStageBar />

      <!-- 上下文用量仪表盘：当前用量 / 阈值线 / 压缩后预估 -->
      <CompressionGauge
        :used-tokens="ctxUsed"
        :max-tokens="ctxMax"
        :threshold-percent="ctxThreshold"
        :after-tokens="ctxAfter"
        label="上下文使用"
      />

        <!-- 压缩效果指标：已压缩 n tokens / 比未压缩前节省 n% / 上一轮压缩大小 n% -->
        <CompressionLevelSteps
          v-if="compressStage === 'idle' || compressStage === 'done'"
          :level="compressLevel"
          :max-level="MAX_LEVEL"
          :base-tokens="compressBaseTokens"
          :current-tokens="compressCurrentTokens"
        />

      <!-- 错误提示 -->
      <div v-if="compressStage === 'error'" class="compress-error-box">
        <div class="compress-error-title">
          <Icon name="warning" :size="16" />
          压缩失败
        </div>
        <div class="compress-error-msg">{{ compressError }}</div>
      </div>

      <!-- 决策统计占比条(done 阶段或已有压缩状态)-->
      <CompressionStatsBar
        v-if="compressStage === 'done' || (compressStage === 'idle' && compressExistingState)"
        :keep="statKeep"
        :hide="statHide"
        :replace="statReplace"
        :total-ids="statTotalIds"
      />

        <!-- 自动压缩设置面板(可展开,保存不重建 agent)-->
      <div class="compress-settings-wrap">
        <button class="compress-settings-toggle" type="button" @click="settingsOpen = !settingsOpen">
          <Icon name="settings" :size="14" />
          <span>自动压缩设置</span>
          <Icon :name="settingsOpen ? 'chevron-up' : 'chevron-down'" :size="12" class="cc-caret" />
        </button>
        <CompressionSettingsPanel
          v-if="settingsOpen && compressionSettings"
          :settings="compressionSettings"
          :saving="compressingSettings"
          @change="onSettingsChange"
        />
      </div>

      <!-- 流式输出区(streaming 阶段实时增长;done 后展示完整 raw_text)-->
      <div
        v-if="compressRawText"
        class="compress-output"
        :class="{ 'is-streaming': compressStage === 'streaming' }"
      >
        <div class="compress-output-title">
          <span>Agent 输出</span>
          <span v-if="compressStage === 'streaming'" class="compress-output-cursor" />
        </div>
        <MarkdownRender :content="compressRawText" />
      </div>

      <!-- 决策列表:streaming(实时) / done(最终) / existing(历史) 三源共用 -->
      <template v-for="src in actionSources" :key="src.key">
        <div
          v-if="src.visible()"
          class="compress-actions"
          :class="{ 'compress-actions-stream': src.key === 'stream' }"
        >
          <div class="compress-actions-title">
            {{ src.title() }}
            <span v-if="src.hint" class="compress-actions-hint">{{ src.hint }}</span>
            <span v-if="src.key === 'existing' && compressExistingState" class="compress-existing-time">
              · {{ new Date(compressExistingState.updated_at).toLocaleString() }}
            </span>
          </div>
          <CompressionActionCard
            v-for="(a, i) in src.actions()"
            :key="`${src.key}-${i}`"
            :action="a"
            :expand-key="`${src.key}-${i}`"
            :expanded="expandedActions.has(`${src.key}-${i}`)"
            :interactive="src.interactive"
            @toggle="toggleActionExpand"
          />
        </div>
      </template>

      <!-- 空状态:未压缩过且 idle -->
      <div v-if="compressStage === 'idle' && !compressExistingState" class="compress-empty">
        <Icon name="merge" :size="32" />
        <div class="compress-empty-text">
          点击"开始压缩"分析当前会话历史，生成 Keep/Hide/Replace 决策以释放上下文空间。
        </div>
      </div>

      <!-- 底部操作区:增强(再次压缩升级 / 清除) -->
      <div class="compress-footer">
        <!-- idle:开始压缩 -->
        <Button
          v-if="compressStage === 'idle'"
          variant="primary"
          block
          :loading="compressing"
          :disabled="compressing"
          @click="triggerCompress"
        >
          <template #icon><Icon name="merge" :size="18" /></template>
          开始压缩
        </Button>

        <!-- done 且未达上限:再次压缩升级到下一级 -->
        <Button
          v-else-if="compressStage === 'done' && !isMaxLevel"
          variant="primary"
          block
          :loading="compressing"
          :disabled="compressing"
          @click="triggerCompress"
        >
          <template #icon><Icon name="merge" :size="18" /></template>
          再次压缩 → 第 {{ compressLevel + 1 }} 级
        </Button>

        <!-- done 且已达上限 -->
        <Button v-else-if="compressStage === 'done' && isMaxLevel" variant="normal" block disabled>
          <Icon name="check" :size="16" />
          已达最高压缩等级（L{{ MAX_LEVEL }}）
        </Button>

        <!-- error:关闭 -->
        <Button v-else-if="compressStage === 'error'" variant="normal" block @click="closeCompressionSheet">
          关闭
        </Button>

        <!-- streaming:进行中 -->
        <Button v-else variant="text" block disabled>
          <Icon name="loader" :size="16" />
          压缩进行中…
        </Button>

        <!-- 清除压缩状态(危险操作,醒目提示) -->
        <Button
          v-if="(compressStage === 'idle' || compressStage === 'done') && hasCompressionState"
          variant="danger"
          block
          @click="clearCompression"
        >
          <Icon name="delete" :size="16" />
          清除压缩状态（恢复全量历史）
        </Button>
      </div>
    </div>
  </BindSheet>
</template>

<style scoped>
/* 与 .ctx-sheet 视觉风格一致,中性 var(--muted) 配色 */
.compress-sheet {
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
  padding: 4px 0 8px;
}

/* 错误提示框 */
.compress-error-box {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 12px 14px;
  background: color-mix(in srgb, var(--danger) 8%, var(--card));
  border: 1px solid color-mix(in srgb, var(--danger) 30%, var(--border));
  border-radius: var(--radius-md);
}

.compress-error-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  font-weight: 600;
  color: var(--danger);
}

.compress-error-msg {
  font-size: 13px;
  color: var(--text);
  word-break: break-word;
}


/* 设置面板:可展开容器 */
.compress-settings-wrap {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.compress-settings-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  cursor: pointer;
  transition: border-color 0.2s ease;
}

.compress-settings-toggle:hover {
  border-color: color-mix(in srgb, var(--primary) 45%, var(--border));
}

.cc-caret {
  margin-left: auto;
  color: var(--muted);
}

/* 流式输出区 */
.compress-output {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.compress-output.is-streaming {
  border-color: color-mix(in srgb, var(--primary) 40%, var(--border));
}

.compress-output-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--muted);
}

.compress-output-cursor {
  width: 6px;
  height: 12px;
  background: var(--accent, #4a7eff);
  display: inline-block;
  animation: compress-cursor-blink 1s step-end infinite;
}

@keyframes compress-cursor-blink {
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
}

/* 决策列表容器 */
.compress-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* 流式实时决策区:带"流式中"指示(穿透卡片组件 scoped) */
.compress-actions-stream :deep(.compress-action-item) {
  border-style: dashed;
  animation: compress-action-fade-in 0.3s ease;
}

@keyframes compress-action-fade-in {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
}

.compress-actions-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.compress-actions-hint {
  font-weight: 400;
  color: var(--muted);
}

/* 上次压缩结果的时间戳 */
.compress-existing-time {
  color: var(--muted);
  font-weight: 400;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

/* 空状态 */
.compress-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 28px 16px;
  color: var(--muted);
}

.compress-empty-text {
  font-size: 13px;
  text-align: center;
  line-height: 1.6;
  max-width: 300px;
}

/* 底部操作区 */
.compress-footer {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: auto;
}
</style>
