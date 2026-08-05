<script setup lang="ts">
/**
 * WorkingDirSheet —— 会话工作区设置 Sheet
 *
 * 展示当前工作区路径,提供选择目录 / 清除操作;并支持把常用目录收藏起来,
 * 以便在「会话工作区」面板一步切换或删除（常用工作区持久化于后端
 * favorite_workspaces.json）。
 * 工作区决定 read_file / list_files / shell 的相对路径基准。
 */
import { inject, watch } from 'vue'
import { Button, Icon, BindSheet } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const {
  workingDirSheetOpen,
  workingDir,
  pickWorkingDir,
  clearWorkingDir,
  favoriteWorkspaces,
  loadFavoriteWorkspaces,
  addFavoriteWorkspace,
  deleteFavoriteWorkspace,
  applyFavoriteWorkspace,
} = store.core

// 每次打开 Sheet 时刷新常用列表
watch(workingDirSheetOpen, (open) => {
  if (open) loadFavoriteWorkspaces()
})
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
        <Button variant="normal" block :disabled="!workingDir" @click="addFavoriteWorkspace(workingDir!)">
          <template #icon><Icon name="star" :size="18" /></template>
          收藏为常用
        </Button>
      </div>

      <div v-if="favoriteWorkspaces.length" class="wd-fav">
        <div class="wd-fav-label">常用工作区</div>
        <div class="wd-fav-list">
          <div
            v-for="ws in favoriteWorkspaces"
            :key="ws.id"
            class="wd-fav-item"
            :class="{ active: ws.path === workingDir }"
          >
            <button class="wd-fav-path" :title="ws.path" @click="applyFavoriteWorkspace(ws.path)">
              {{ ws.path }}
            </button>
            <button class="wd-fav-del" title="删除常用" @click="deleteFavoriteWorkspace(ws.id)">
              <Icon name="delete" :size="14" />
            </button>
          </div>
        </div>
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
  border-radius: var(--radius-lg);
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

.wd-fav {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.wd-fav-label {
  font-size: 12px;
  color: var(--muted);
}

.wd-fav-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 180px;
  overflow-y: auto;
}

.wd-fav-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.wd-fav-item.active {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, var(--bg-2));
}

.wd-fav-path {
  flex: 1;
  min-width: 0;
  text-align: left;
  font-size: 13px;
  color: var(--text);
  background: none;
  border: none;
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wd-fav-path:hover {
  color: var(--accent);
}

.wd-fav-del {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  color: var(--muted);
  background: none;
  border: none;
  border-radius: 5px;
  cursor: pointer;
}

.wd-fav-del:hover {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 12%, transparent);
}

.wd-hint {
  font-size: 12px;
  color: var(--muted);
  line-height: 1.6;
  margin: 0;
}
</style>
