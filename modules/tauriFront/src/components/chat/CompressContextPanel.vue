<script setup lang="ts">
/**
 * CompressContextPanel —— 右栏「压缩上下文」页签编排壳（原子文件）
 *
 * 复用 store.compression（useChatEvents 实时监听 agent-compress-* 事件持续更新）
 * 与压缩原子组件，把右栏从「简陋设置 + 简单列表」升级为数据仪表盘：
 *  - 压缩效果指标卡（CompressionLevelSteps：已压缩 n tokens / 节省 n% / 上一轮压缩大小 n%)
 *  - 上下文用量仪表盘（当前用量 / 阈值线 / 压缩后预估，一眼看出能省多少）
 *  - 实时阶段进度条（loading → building → streaming → parsing → persisting → done）
 *  - 决策统计占比条（keep/hide/replace 堆叠 + 图例）
 *  - 流式输出（streaming 自动展开，可折叠避免撑爆右栏）
 *  - 决策列表（streaming 实时 / done 最终 / existing 历史 三源共用）
 *  - 操作区（开始压缩 / 再次升级 / 清除，随状态动态切换）
 *
 * 反馈速度：右栏打开即可看到实时进度与流式决策，无需等待命令返回。
 * 本组件只做编排，卡片级 UI 全部来自已有原子组件，遵循单一原子文件原则。
 */
import { computed, inject, nextTick, ref, watch } from 'vue'
import MarkdownRender from 'markstream-vue'
import { Icon, Button } from '../basic'
import CompressionStageBar from './CompressionStageBar.vue'
import CompressionActionCard from './CompressionActionCard.vue'
import CompressionGauge from './CompressionGauge.vue'
import CompressionLevelSteps from './CompressionLevelSteps.vue'
import CompressionStatsBar from './CompressionStatsBar.vue'
import CompressionSettingsPanel from './CompressionSettingsPanel.vue'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import type { CompressionSettings } from '../../types'

const store = inject(CHAT_STORE_KEY)!
const { core, compression } = store
const {
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
  clearCompression,
  loadCompressionSettings,
  saveCompressionSettings,
} = compression

/** 最高压缩等级（与后端 MAX_COMPRESSION_LEVEL 对齐） */
const MAX_LEVEL = 3

// ---------- 上下文用量仪表盘 ----------
const ctxUsed = computed(() => core.contextUsedTokens.value)
const ctxMax = computed(() => core.contextMaxTokens.value)
const ctxThreshold = computed(() => compressionSettings.value?.threshold_percent ?? 80)
// 压缩后预估用量 = 当前用量 - 已节省 tokens（无节省数据时为 null）
const ctxAfter = computed(() => {
  const saved = compressSavedInfo.value?.savedTokens ?? 0
  if (saved <= 0) return null
  return Math.max(0, ctxUsed.value - saved)
})

// 是否存在压缩状态（清除按钮的依据）
const hasCompressionState = computed(
  () => !!compressExistingState.value || compressActions.value.length > 0,
)
const isMaxLevel = computed(() => compressLevel.value >= MAX_LEVEL)

// 统计展示：done 用本轮结果，idle + existing 用既有状态
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

// ---------- 自动压缩设置（折叠） ----------
const settingsOpen = ref(false)
async function onSettingsChange(s: CompressionSettings) {
  await saveCompressionSettings(s)
}

// ---------- 流式输出（可折叠） ----------
const outputOpen = ref(false)
watch(
  () => compressStage.value,
  (s) => {
    // streaming 自动展开让用户实时看到 Agent 决策；结束/失败收起避免撑爆右栏
    if (s === 'streaming') outputOpen.value = true
    if (s === 'done' || s === 'error') outputOpen.value = false
  },
)
function toggleOutput() {
  outputOpen.value = !outputOpen.value
}

// ---------- 三源决策列表（与 CompressionSheet 一致） ----------
const actionSources = [
  {
    key: 'stream' as const,
    visible: () => compressStage.value === 'streaming' && streamParsedActions.value.length > 0,
    title: () => `实时决策（${streamParsedActions.value.length} 条）`,
    hint: '已解析',
    actions: () => streamParsedActions.value,
    interactive: false,
  },
  {
    key: 'done' as const,
    visible: () => compressStage.value === 'done' && compressActions.value.length > 0,
    title: () => `压缩决策（第 ${compressLevel.value} 级）`,
    hint: '点击展开原文',
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

// ---------- 触发压缩：把阶段进度条滚进视野，保证反馈即时可见 ----------
const stageAnchor = ref<HTMLElement | null>(null)
function onTrigger() {
  void triggerCompress()
  nextTick(() => {
    stageAnchor.value?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  })
}

// 兜底：确保设置已加载（ChatWindow 已加载过一次，此处为右栏独立挂载时保险）
if (compressionSettings.value == null) void loadCompressionSettings()
</script>

<template>
  <div class="ccp">
    <!-- 压缩效果指标：已压缩 n tokens / 节省 n% / 上一轮压缩大小 n% -->
      <CompressionLevelSteps
        v-if="compressStage === 'idle' || compressStage === 'done'"
        :level="compressLevel"
        :max-level="MAX_LEVEL"
        :base-tokens="compressBaseTokens"
        :current-tokens="compressCurrentTokens"
      />

    <!-- 上下文用量仪表盘：当前用量 / 阈值线 / 压缩后预估 -->
    <CompressionGauge
      :used-tokens="ctxUsed"
      :max-tokens="ctxMax"
      :threshold-percent="ctxThreshold"
      :after-tokens="ctxAfter"
      label="上下文使用"
    />

      <!-- 实时阶段进度条（反馈速度核心，滚动锚点；idle 无压缩活动时不展示） -->
      <div ref="stageAnchor">
        <CompressionStageBar v-if="compressStage !== 'idle'" />
      </div>

    <!-- 错误提示 -->
    <div v-if="compressStage === 'error'" class="ccp-error">
      <div class="ccp-error-title">
        <Icon name="warning" :size="15" />
        压缩失败
      </div>
      <div class="ccp-error-msg">{{ compressError }}</div>
    </div>

    <!-- 决策统计占比条：done 阶段或已有压缩状态 -->
    <CompressionStatsBar
      v-if="compressStage === 'done' || (compressStage === 'idle' && compressExistingState)"
      :keep="statKeep"
      :hide="statHide"
      :replace="statReplace"
      :total-ids="statTotalIds"
    />

    <!-- 压缩前后对比：压缩前 tokens → 压缩后 tokens + 节省率 -->
    <div
      v-if="compressSavedInfo && compressSavedInfo.savedTokens > 0"
      class="ccp-compare"
      :class="{ 'is-done': compressStage === 'done' }"
    >
      <span class="ccp-compare-icon">
        <Icon name="check" :size="13" />
      </span>
      <div class="ccp-compare-body">
        <div class="ccp-compare-line">
          <span class="ccp-before">{{ ctxUsed.toLocaleString('zh-CN') }}</span>
          <Icon name="arrow-right" :size="12" class="ccp-arrow" />
          <b class="ccp-after">{{ (ctxAfter ?? 0).toLocaleString('zh-CN') }}</b>
          <span class="ccp-unit">tokens</span>
        </div>
        <div class="ccp-compare-save">
          释放 ↓ {{ compressSavedInfo.savedTokens.toLocaleString('zh-CN') }} tokens ·
          {{ compressSavedInfo.percent }}%
        </div>
      </div>
    </div>

    <!-- 自动压缩设置（可折叠） -->
    <div class="ccp-settings-wrap">
      <button type="button" class="ccp-settings-toggle" @click="settingsOpen = !settingsOpen">
        <Icon name="settings" :size="14" />
        <span>自动压缩设置</span>
        <Icon :name="settingsOpen ? 'chevron-up' : 'chevron-down'" :size="12" class="ccp-caret" />
      </button>
      <CompressionSettingsPanel
        v-if="settingsOpen && compressionSettings"
        :settings="compressionSettings"
        :saving="compressingSettings"
        @change="onSettingsChange"
      />
    </div>

    <!-- 流式输出区（streaming 自动展开，可折叠） -->
    <div
      v-if="compressRawText"
      class="ccp-output"
      :class="{ 'is-streaming': compressStage === 'streaming' }"
    >
      <button type="button" class="ccp-output-head" @click="toggleOutput">
        <Icon name="merge" :size="13" />
        <span>Agent 输出</span>
        <span class="ccp-output-len">{{ compressRawText.length }} 字</span>
        <Icon v-if="compressStage === 'streaming'" name="loader" :size="12" class="ccp-output-cursor" />
        <Icon :name="outputOpen ? 'chevron-up' : 'chevron-down'" :size="12" class="ccp-caret" />
      </button>
      <div v-if="outputOpen" class="ccp-output-body">
        <MarkdownRender :content="compressRawText" />
      </div>
    </div>

    <!-- 决策列表：streaming 实时 / done 最终 / existing 历史 三源共用 -->
    <template v-for="src in actionSources" :key="src.key">
      <div v-if="src.visible()" class="ccp-actions">
        <div class="ccp-actions-title">
          {{ src.title() }}
          <span v-if="src.hint" class="ccp-actions-hint">{{ src.hint }}</span>
          <span v-if="src.key === 'existing' && compressExistingState" class="ccp-existing-time">
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

    <!-- 空状态：未压缩过且 idle -->
    <div v-if="compressStage === 'idle' && !compressExistingState" class="ccp-empty">
      <Icon name="merge" :size="26" />
      <div>点击下方按钮分析会话历史，生成 Keep / Hide / Replace 决策，释放上下文空间。</div>
    </div>

    <!-- 操作区：随状态动态切换 -->
    <div class="ccp-footer">
      <!-- idle：开始压缩 -->
      <Button
        v-if="compressStage === 'idle'"
        variant="primary"
        block
        :loading="compressing"
        :disabled="compressing"
        @click="onTrigger"
      >
        <template #icon><Icon name="merge" :size="16" /></template>
        开始压缩
      </Button>

      <!-- done 且未达上限：再次压缩升级 -->
      <Button
        v-else-if="compressStage === 'done' && !isMaxLevel"
        variant="primary"
        block
        :loading="compressing"
        :disabled="compressing"
        @click="onTrigger"
      >
        <template #icon><Icon name="merge" :size="16" /></template>
        再次压缩 → 第 {{ compressLevel + 1 }} 级
      </Button>

      <!-- done 且已达上限 -->
      <Button v-else-if="compressStage === 'done' && isMaxLevel" variant="normal" block disabled>
        <Icon name="check" :size="15" />
        已达最高压缩等级（L{{ MAX_LEVEL }}）
      </Button>

      <!-- error：关闭输出区 -->
      <Button v-else-if="compressStage === 'error'" variant="normal" block @click="outputOpen = false">
        <Icon name="close" :size="15" />
        收起
      </Button>

      <!-- streaming：进行中 -->
      <Button v-else variant="text" block disabled>
        <Icon name="loader" :size="15" />
        压缩进行中…
      </Button>

      <!-- 清除压缩状态（危险操作） -->
      <Button
        v-if="(compressStage === 'idle' || compressStage === 'done') && hasCompressionState"
        variant="danger"
        block
        @click="clearCompression"
      >
        <Icon name="delete" :size="15" />
        清除压缩状态（恢复全量历史）
      </Button>
    </div>
  </div>
</template>

<style scoped>
.ccp {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* ---------- 错误提示 ---------- */
.ccp-error {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  background: color-mix(in srgb, var(--danger) 8%, var(--bg-2));
  border: 1px solid color-mix(in srgb, var(--danger) 30%, var(--border));
  border-radius: var(--radius-md);
}
.ccp-error-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--danger);
}
.ccp-error-msg {
  font-size: 12px;
  color: var(--text);
  word-break: break-word;
  line-height: 1.5;
}

/* ---------- 压缩前后对比 ---------- */
.ccp-compare {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--success) 8%, var(--bg-2));
  border: 1px solid color-mix(in srgb, var(--success) 25%, var(--border));
  transition: all 0.3s ease;
}
.ccp-compare.is-done {
  animation: ccp-saved-pop 0.4s ease;
}
@keyframes ccp-saved-pop {
  0% { transform: scale(0.96); opacity: 0; }
  60% { transform: scale(1.02); }
  100% { transform: scale(1); opacity: 1; }
}
.ccp-compare-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--success) 14%, transparent);
  color: var(--success);
  flex-shrink: 0;
}
.ccp-compare-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.ccp-compare-line {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 13px;
  color: var(--text);
  font-variant-numeric: tabular-nums;
  flex-wrap: wrap;
}
.ccp-before {
  color: var(--muted);
  text-decoration: line-through;
}
.ccp-arrow {
  color: var(--muted);
}
.ccp-after {
  font-weight: 700;
  color: var(--success);
}
.ccp-unit {
  color: var(--muted);
  font-size: 11px;
}
.ccp-compare-save {
  font-size: 11px;
  color: var(--success);
  font-weight: 600;
}

/* ---------- 设置面板：可展开容器 ---------- */
.ccp-settings-wrap {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.ccp-settings-toggle {
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
.ccp-settings-toggle:hover {
  border-color: color-mix(in srgb, var(--primary) 45%, var(--border));
}
.ccp-caret {
  margin-left: auto;
  color: var(--muted);
}

/* ---------- 流式输出区 ---------- */
.ccp-output {
  display: flex;
  flex-direction: column;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}
.ccp-output.is-streaming {
  border-color: color-mix(in srgb, var(--primary) 40%, var(--border));
}
.ccp-output-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  border: none;
  background: transparent;
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
  cursor: pointer;
  text-align: left;
}
.ccp-output-head:hover {
  background: var(--hover);
}
.ccp-output-len {
  color: var(--muted);
  font-weight: 400;
  font-variant-numeric: tabular-nums;
}
.ccp-output-cursor {
  width: 5px;
  height: 10px;
  background: var(--accent, #4a7eff);
  display: inline-block;
  animation: ccp-cursor-blink 1s step-end infinite;
}
@keyframes ccp-cursor-blink {
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
}
.ccp-output-body {
  padding: 8px 12px 12px;
  border-top: 1px dashed var(--border);
  max-height: 220px;
  overflow-y: auto;
  font-size: 12px;
}

/* ---------- 决策列表 ---------- */
.ccp-actions {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.ccp-actions-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
}
.ccp-actions-hint {
  font-weight: 400;
  color: var(--muted);
}
.ccp-existing-time {
  color: var(--muted);
  font-weight: 400;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

/* ---------- 空状态 ---------- */
.ccp-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 20px 12px;
  color: var(--muted);
  font-size: 12px;
  text-align: center;
  line-height: 1.6;
}

/* ---------- 操作区 ---------- */
.ccp-footer {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 2px;
}
</style>
