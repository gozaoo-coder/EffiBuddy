<script setup lang="ts">
/**
 * ToolSheet —— 底部工具/附件 Sheet
 *
 * 拍照 / 照片 / 本地文件接真实 command,其余提示即将上线;
 * 底部提供会话级工作区入口。
 */
import { inject } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Icon, BindSheet, useToast } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import type { PickedFile } from '../../types'

const store = inject(CHAT_STORE_KEY)!
const { toolSheetOpen, workingDir, workingDirSheetOpen, formatFileSize } = store.core
const { toast } = useToast()

const toolCategories = [
  { label: '拍照', icon: 'camera' },
  { label: '照片', icon: 'image' },
  { label: '本地文件', icon: 'folder' },
  { label: '微信文件', icon: 'wechat' },
]

const pluginItems = [
  { label: '插件', desc: '接入 App 和数据库，帮你自动操作', icon: 'plug' },
  { label: '技能', desc: '复用专业能力，稳定处理特定任务', icon: 'tool' },
]

// 工具卡片点击:拍照/照片/本地文件接真实 command
async function onToolClick(label: string) {
  toolSheetOpen.value = false
  try {
    let file: PickedFile | null = null
    if (label === '拍照') {
      file = await invoke<PickedFile>('capture_photo')
    } else if (label === '照片') {
      file = await invoke<PickedFile>('pick_image')
    } else if (label === '本地文件') {
      file = await invoke<PickedFile>('pick_file')
    } else {
      toast({ content: `${label} 功能即将上线`, type: 'info' })
      return
    }
    if (file) {
      toast({
        content: `已选择：${file.name}（${formatFileSize(file.size)}）`,
        type: 'success',
      })
    }
  } catch (e) {
    toast({ content: `${label}失败：${e}`, type: 'error' })
  }
}
</script>

<template>
  <BindSheet v-model:visible="toolSheetOpen" title="工具" side="bottom" :height="'auto'">
    <div class="tool-sheet">
      <div class="tool-row">
        <div v-for="t in toolCategories" :key="t.label" class="tool-card" @click="onToolClick(t.label)">
          <span class="tool-card-icon"><Icon :name="t.icon" :size="24" /></span>
          <span class="tool-card-label">{{ t.label }}</span>
        </div>
      </div>

      <div class="tool-list">
        <div v-for="p in pluginItems" :key="p.label" class="tool-list-item" @click="onToolClick(p.label)">
          <span class="tool-list-icon"><Icon :name="p.icon" :size="20" /></span>
          <div class="tool-list-text">
            <div class="tool-list-title">{{ p.label }}</div>
            <div class="tool-list-desc">{{ p.desc }}</div>
          </div>
          <span class="tool-list-arrow"><Icon name="chevron-right" :size="16" /></span>
        </div>
      </div>

      <div class="tool-list-item" @click="onToolClick('联网搜索')">
        <span class="tool-list-icon"><Icon name="globe" :size="20" /></span>
        <div class="tool-list-text">
          <div class="tool-list-title">联网搜索</div>
        </div>
        <span class="tool-list-status">自动</span>
        <span class="tool-list-arrow"><Icon name="chevron-right" :size="16" /></span>
      </div>

      <!-- 会话级工作区入口:read_file/list_files/shell 以此为基准 -->
      <div
        class="tool-list-item"
        :title="workingDir ? workingDir : '未设置，使用技能级或默认'"
        @click="workingDirSheetOpen = true"
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
    </div>
  </BindSheet>
</template>

<style scoped>
.tool-sheet {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 4px 0 8px;
}

.tool-row {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 10px;
  padding: 8px 0 12px;
}

.tool-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 16px 8px;
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, transform 0.15s ease;
}

.tool-card:hover {
  background: color-mix(in srgb, var(--primary) 8%, var(--card));
  border-color: color-mix(in srgb, var(--primary) 40%, var(--border));
  transform: translateY(-1px);
}

.tool-card-icon {
  display: inline-flex;
  color: var(--primary);
}

.tool-card-label {
  font-size: 13px;
  color: var(--text);
}

.tool-list {
  display: flex;
  flex-direction: column;
}

.tool-list-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background 0.15s ease;
}

.tool-list-item:last-child {
  border-bottom: none;
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
  border-radius: var(--radius-lg);
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
