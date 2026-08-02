<script setup lang="ts">
/**
 * ChatContextSheet —— 上下文管理 Sheet
 *
 * 上下文使用统计 + 消息压缩按钮 + 工作区入口。
 */
import { inject } from 'vue'
import { Button, Icon, BindSheet, ContextRing } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const {
  contextSheetOpen,
  contextUsedTokens,
  contextMaxTokens,
  contextUsedChars,
  workingDir,
  workingDirSheetOpen,
} = store.core
const { compressing, triggerCompress } = store.compression
</script>

<template>
  <BindSheet
    v-model:visible="contextSheetOpen"
    title="上下文管理"
    side="bottom"
    :height="'auto'"
  >
    <div class="ctx-sheet">
      <!-- 上下文使用统计 -->
      <div class="ctx-stat">
        <div class="ctx-stat-row">
          <ContextRing :used="contextUsedTokens" :max="contextMaxTokens" :size="32" />
          <div class="ctx-stat-text">
            <div class="ctx-stat-title">上下文使用</div>
            <div class="ctx-stat-desc">
              {{ contextUsedTokens }} / {{ contextMaxTokens }} tokens · 约
              {{ contextUsedChars }} 字符
            </div>
          </div>
        </div>
      </div>

      <!-- 消息压缩按钮:打开浮窗展示进度 -->
      <Button
        variant="primary"
        block
        :loading="compressing"
        :disabled="compressing"
        @click="triggerCompress"
      >
        <template #icon><Icon name="merge" :size="18" /></template>
        {{ compressing ? '压缩中…' : '压缩消息' }}
      </Button>

      <!-- 工作区显示(点击调出 workingDirSheet)-->
      <div
        class="tool-list-item"
        :title="workingDir ?? '未设置'"
        @click="contextSheetOpen = false; workingDirSheetOpen = true"
      >
        <span class="tool-list-icon"><Icon name="folder" :size="20" /></span>
        <div class="tool-list-text">
          <div class="tool-list-title">工作区</div>
          <div class="tool-list-desc">
            {{ workingDir ? workingDir : '未设置，相对路径以默认目录为准' }}
          </div>
        </div>
        <span class="tool-list-status">{{ workingDir ? '已设置' : '默认' }}</span>
        <span class="tool-list-arrow"><Icon name="chevron-right" :size="16" /></span>
      </div>

      <p class="ctx-hint">
        压缩消息会合并历史对话以释放上下文空间。工作区决定 read_file / list_files / shell 的相对路径基准。
      </p>
    </div>
  </BindSheet>
</template>

<style scoped>
.ctx-sheet {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 4px 0 8px;
}

.ctx-stat {
  display: flex;
  align-items: center;
  padding: 8px 12px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.ctx-stat-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.ctx-stat-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.ctx-stat-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}

.ctx-stat-desc {
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}

.ctx-hint {
  font-size: 12px;
  color: var(--muted);
  line-height: 1.6;
  margin: 0;
}

/* 与 ToolSheet 共享的列表项样式(Sheet 内复用的 tool-list 视觉) */
.tool-list-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background 0.15s ease;
}

.tool-list-item:hover {
  background: var(--bg-2);
}

.tool-list-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--primary) 10%, var(--card));
  color: var(--primary);
  flex-shrink: 0;
}

.tool-list-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.tool-list-title {
  font-size: 14px;
  font-weight: 500;
  color: var(--text);
}

.tool-list-desc {
  font-size: 12px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tool-list-status {
  font-size: 12px;
  color: var(--muted);
  flex-shrink: 0;
}

.tool-list-arrow {
  color: var(--muted);
  display: inline-flex;
  flex-shrink: 0;
}
</style>
