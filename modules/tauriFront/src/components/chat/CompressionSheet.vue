<script setup lang="ts">
/**
 * CompressionSheet —— 消息压缩进度浮窗(编排壳)
 *
 * 区块编排:阶段进度条(CompressionStageBar)、错误提示、决策统计、
 * Token 节省卡片、流式输出、决策列表(CompressionActionCard × 3 种数据源)、
 * 空状态与底部操作区。
 * 卡片级 UI 与阶段条 UI 已原子化为子组件,本文件只保留容器与编排逻辑。
 */
import { inject } from 'vue'
import MarkdownRender from 'markstream-vue'
import { Button, Icon, BindSheet } from '../basic'
import CompressionStageBar from './CompressionStageBar.vue'
import CompressionActionCard from './CompressionActionCard.vue'
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
  toggleActionExpand,
  triggerCompress,
  closeCompressionSheet,
  clearCompression,
} = store.compression

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
    title: () => `压缩决策（${compressActions.value.length} 条 · 点击展开查看原文）`,
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
    title: () => '上次压缩结果',
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

      <!-- 错误提示 -->
      <div v-if="compressStage === 'error'" class="compress-error-box">
        <div class="compress-error-title">
          <Icon name="warning" :size="16" />
          压缩失败
        </div>
        <div class="compress-error-msg">{{ compressError }}</div>
      </div>

      <!-- 决策统计(done 阶段或已有压缩状态)-->
      <div
        v-if="compressStage === 'done' || (compressStage === 'idle' && compressExistingState)"
        class="compress-stats"
      >
        <div class="compress-stat-item keep">
          <span class="compress-stat-num">{{
            compressStage === 'done'
              ? compressActionStats.keep
              : (compressExistingState?.actions.filter((a) => a.method === 'keep').length ?? 0)
          }}</span>
          <span class="compress-stat-label">保持</span>
        </div>
        <div class="compress-stat-item hide">
          <span class="compress-stat-num">{{
            compressStage === 'done'
              ? compressActionStats.hide
              : (compressExistingState?.actions.filter((a) => a.method === 'hide').length ?? 0)
          }}</span>
          <span class="compress-stat-label">隐藏</span>
        </div>
        <div class="compress-stat-item replace">
          <span class="compress-stat-num">{{
            compressStage === 'done'
              ? compressActionStats.replace
              : (compressExistingState?.actions.filter((a) => a.method === 'replace').length ?? 0)
          }}</span>
          <span class="compress-stat-label">替换</span>
        </div>
        <div class="compress-stat-item total">
          <span class="compress-stat-num">{{
            compressStage === 'done'
              ? compressActionStats.totalIds
              : (compressExistingState?.actions.reduce((s, a) => s + a.message_ids.length, 0) ?? 0)
          }}</span>
          <span class="compress-stat-label">涉及消息</span>
        </div>
      </div>

      <!-- Token 节省量卡片:基于 actions + messages 估算(4 字符 ≈ 1 token)-->
      <div
        v-if="compressSavedInfo && compressSavedInfo.savedTokens > 0"
        class="compress-saved"
        :class="{ 'is-done': compressStage === 'done' }"
      >
        <div class="compress-saved-icon">
          <Icon name="check" :size="14" />
        </div>
        <div class="compress-saved-text">
          <div class="compress-saved-title">
            节省约 <span class="compress-saved-num">{{ compressSavedInfo.savedTokens }}</span> tokens
          </div>
          <div class="compress-saved-desc">
            {{ compressSavedInfo.savedChars }} 字符 · 占历史 {{ compressSavedInfo.percent }}%
          </div>
        </div>
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

      <!-- 底部操作区 -->
      <div class="compress-footer">
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
        <Button
          v-else-if="compressStage === 'done' || compressStage === 'error'"
          variant="normal"
          block
          @click="closeCompressionSheet"
        >
          关闭
        </Button>
        <Button v-else variant="text" block disabled>
          <Icon name="loader" :size="16" />
          压缩进行中…
        </Button>
        <Button
          v-if="(compressStage === 'idle' || compressStage === 'done') && compressExistingState"
          variant="text"
          block
          @click="clearCompression"
        >
          <Icon name="delete" :size="16" />
          清除压缩状态
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

/* 决策统计:四宫格 */
.compress-stats {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
}

.compress-stat-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 10px 4px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.compress-stat-num {
  font-size: 18px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}

.compress-stat-label {
  font-size: 12px;
  color: var(--muted);
}

.compress-stat-item.keep .compress-stat-num { color: var(--success, #10a37f); }
.compress-stat-item.hide .compress-stat-num { color: var(--muted); }
.compress-stat-item.replace .compress-stat-num { color: var(--warn, #d97757); }
.compress-stat-item.total .compress-stat-num { color: var(--text); }

/* Token 节省量卡片:横排图标 + 标题 + 描述,配色用 success */
.compress-saved {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  background: rgba(16, 163, 127, 0.08);
  border: 1px solid rgba(16, 163, 127, 0.25);
  transition: all 0.3s ease;
}

.compress-saved.is-done {
  animation: compress-saved-pop 0.4s ease;
}

@keyframes compress-saved-pop {
  0% { transform: scale(0.96); opacity: 0; }
  60% { transform: scale(1.02); }
  100% { transform: scale(1); opacity: 1; }
}

.compress-saved-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--success) 14%, transparent);
  color: var(--success);
  flex-shrink: 0;
}

.compress-saved-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.compress-saved-title {
  font-size: 13px;
  color: var(--text);
}

.compress-saved-num {
  font-weight: 700;
  color: var(--success);
  font-variant-numeric: tabular-nums;
}

.compress-saved-desc {
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
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
