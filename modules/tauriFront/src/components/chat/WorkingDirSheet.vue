<script setup lang="ts">
/**
 * WorkingDirSheet —— 会话工作区设置 Sheet
 *
 * 展示当前工作区路径,提供选择目录 / 清除操作。
 * 工作区决定 read_file / list_files / shell 的相对路径基准。
 */
import { inject } from 'vue'
import { Button, Icon, BindSheet } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const { workingDirSheetOpen, workingDir, pickWorkingDir, clearWorkingDir } = store.core
</script>

<template>
  <BindSheet v-model:visible="workingDirSheetOpen" title="会话工作区" side="bottom" :height="'auto'">
    <div class="wd-sheet">
      <div class="wd-current">
        <div class="wd-current-label">当前工作区</div>
        <div class="wd-current-path" :class="{ 'is-empty': !workingDir }">
          {{ workingDir || '未设置（使用技能级或进程默认目录）' }}
        </div>
      </div>
      <div class="wd-actions">
        <Button variant="primary" block @click="pickWorkingDir">
          <template #icon><Icon name="folder" :size="18" /></template>
          选择目录
        </Button>
        <Button variant="normal" block :disabled="!workingDir" @click="clearWorkingDir">
          清除工作区
        </Button>
      </div>
      <p class="wd-hint">
        工作区决定 read_file / list_files / shell 的相对路径基准与命令执行目录。
        优先级：会话级 &gt; 技能级 &gt; 进程默认。
      </p>
    </div>
  </BindSheet>
</template>

<style scoped>
.wd-sheet {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 4px 0 8px;
}

.wd-current {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px 14px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.wd-current-label {
  font-size: 12px;
  color: var(--muted);
}

.wd-current-path {
  font-size: 14px;
  color: var(--text);
  word-break: break-all;
  line-height: 1.5;
}

.wd-current-path.is-empty {
  color: var(--muted);
}

.wd-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.wd-hint {
  font-size: 12px;
  color: var(--muted);
  line-height: 1.6;
  margin: 0;
}
</style>
