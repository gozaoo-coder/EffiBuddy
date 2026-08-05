<script setup lang="ts">
/**
 * AsrUploadTab —— ASR 文件上传转写视图
 *
 * 全流程管线：
 *   idle →（选择/拖入文件）→ selected →（开始转写）→ transcribing → done
 *                                                      ↘ error
 *   中断：转写中切走页签不阻塞（后端独立运行），返回后凭 record_id 继续展示进度。
 *
 * 文件获取：
 *   - 点击选择：invoke('pick_file') 拿到本地路径（系统对话框）
 *   - 拖入：getCurrentWebview().onDragDropEvent 拿到真实路径（Tauri 2 默认拦截 HTML5 drop）
 *
 * 进度：监听 asr-upload-progress 事件，progress 0–1 平滑过渡。
 */
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { animate } from 'animejs'
import Icon from '../Icon.vue'
import { Button, Dropdown, useToast } from '../basic'
import { useAsr } from '../../composables/useAsr'
import { useTabs } from '../../composables/useTabs'
import type {
  AsrFinishResult,
  AsrUploadProgressPayload,
  PickedFile,
  TabItem,
} from '../../types'

defineOptions({ name: 'AsrUploadTab' })

const props = defineProps<{
  tab: TabItem
}>()

const emit = defineEmits<{
  (e: 'update:status', status: TabItem['status']): void
}>()

const { toast } = useToast()
const { transcribeFile, onUploadProgress } = useAsr()
const { openTab } = useTabs()

type Phase = 'idle' | 'selected' | 'transcribing' | 'done' | 'error'
const phase = ref<Phase>('idle')
const pickedPath = ref('')
const pickedName = ref('')
const pickedSize = ref(0)
const errorMessage = ref('')
const result = ref<AsrFinishResult | null>(null)
const progress = ref(0) // 0–1
const progressStatus = ref('')
const dragHover = ref(false)

// 语言选项
const langOptions = [
  { value: 'zh-CN', label: '中文（普通话）' },
  { value: 'en-US', label: '英语（美国）' },
  { value: 'ja-JP', label: '日语' },
  { value: 'ko-KR', label: '韩语' },
  { value: '', label: '自动检测' },
]
const lang = ref('zh-CN')

const SUPPORTED_EXT = ['wav', 'mp3', 'm4a', 'ogg', 'flac', 'aac', 'mp4']

const fileExt = computed(() => {
  const n = pickedName.value
  const idx = n.lastIndexOf('.')
  return idx >= 0 ? n.slice(idx + 1).toLowerCase() : ''
})

const sizeDisplay = computed(() => {
  const b = pickedSize.value
  if (b <= 0) return ''
  if (b < 1024) return `${b} B`
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`
  return `${(b / (1024 * 1024)).toFixed(2)} MB`
})

const progressPct = computed(() => Math.round(progress.value * 100))

// ============= 动画 =============
const resultRef = ref<HTMLElement | null>(null)

/** 结果进入：淡入 + 上移 */
function animateResultIn(): void {
  const el = resultRef.value
  if (!el) return
  animate(el, {
    opacity: [0, 1],
    translateY: [12, 0],
    duration: 320,
    ease: 'out(3)',
  })
}

// 转写完成时触发结果进入动画
watch(phase, async (p) => {
  if (p === 'done') {
    await nextTick()
    animateResultIn()
  }
})

// ============= 文件选择 =============
async function pickFile(): Promise<void> {
  if (phase.value === 'transcribing') return
  try {
    const picked = await invoke<PickedFile | null>('pick_file')
    if (!picked) return // 用户取消
    acceptPicked(picked)
  } catch (e) {
    toast({ content: `选择文件失败：${e}`, type: 'error' })
  }
}

function acceptPicked(p: PickedFile): void {
  const ext = p.name.split('.').pop()?.toLowerCase() ?? ''
  if (!SUPPORTED_EXT.includes(ext)) {
    toast({
      content: `不支持的格式 .${ext}，支持 ${SUPPORTED_EXT.join(' / ')}`,
      type: 'warn',
    })
    return
  }
  pickedPath.value = p.path
  pickedName.value = p.name
  pickedSize.value = p.size
  progress.value = 0
  progressStatus.value = ''
  result.value = null
  errorMessage.value = ''
  phase.value = 'selected'
  emit('update:status', 'idle')
}

/** 清除已选文件 */
function clearPicked(): void {
  pickedPath.value = ''
  pickedName.value = ''
  pickedSize.value = 0
  phase.value = 'idle'
}

// ============= 拖拽（Tauri 原生事件，拿真实路径） =============
type DragDropPayload = {
  type: 'enter' | 'over' | 'drop' | 'leave'
  paths: string[]
  position: { x: number; y: number }
}

function onDragDropEvent(payload: DragDropPayload): void {
  if (phase.value === 'transcribing') return
  if (payload.type === 'enter' || payload.type === 'over') {
    dragHover.value = true
  } else if (payload.type === 'leave') {
    dragHover.value = false
  } else if (payload.type === 'drop') {
    dragHover.value = false
    const path = payload.paths?.[0]
    if (!path) return
    const name = path.split(/[\\/]/).pop() ?? path
    // 路径来自系统，size 未知；后端 transcribe_file 会校验存在性
    acceptPicked({ path, name, size: 0 })
  }
}

let unlistenDrag: (() => void) | null = null

// ============= 转写流程 =============
let unsubProgress: (() => void) | null = null

async function startTranscribe(): Promise<void> {
  if (!pickedPath.value || phase.value === 'transcribing') return
  phase.value = 'transcribing'
  progress.value = 0
  progressStatus.value = '初始化…'
  emit('update:status', 'loading')

  try {
    const res = await transcribeFile(pickedPath.value, lang.value || null)
    result.value = res
    progress.value = 1
    progressStatus.value = '完成'
    phase.value = 'done'
    emit('update:status', 'active')
    toast({
      content: res.summary ? '转写完成，已生成摘要' : '转写完成',
      type: 'success',
    })
  } catch (e) {
    errorMessage.value = String(e)
    phase.value = 'error'
    emit('update:status', 'error')
    toast({ content: `转写失败：${e}`, type: 'error' })
  }
}

function gotoHistory(): void {
  openTab({
    id: 'asr-history',
    kind: 'asr-history',
    title: 'ASR 历史',
    closable: true,
    instanceKey: '',
  })
}

// ============= 生命周期 =============
function init(): void {
  emit('update:status', 'idle')
  unsubProgress = onUploadProgress((p: AsrUploadProgressPayload) => {
    progress.value = Math.max(0, Math.min(1, p.progress))
    progressStatus.value = p.status
  })
  // 拖拽事件：Tauri 2 默认拦截 HTML5 drop，需用原生事件拿路径
  if (typeof getCurrentWebview === 'function') {
    getCurrentWebview()
      .onDragDropEvent((e) => {
        onDragDropEvent(e.payload as DragDropPayload)
      })
      .then((un: () => void) => {
        unlistenDrag = un
      })
      .catch(() => {
        /* 拖拽不可用时仅支持点击选择 */
      })
  }
}

function destroy(): void {
  /* 转写由后端独立运行，无需中断 */
}

onMounted(() => {
  init()
})

onBeforeUnmount(() => {
  destroy()
  unsubProgress?.()
  unlistenDrag?.()
})

defineExpose({ init, destroy })
</script>

<template>
  <div class="asr-tab asr-upload-tab">
    <div class="upload-head">
      <div class="upload-badge">
        <Icon name="attachment" :size="28" />
      </div>
      <div class="upload-head-text">
        <h2 class="upload-title">ASR 文件转写</h2>
        <p class="upload-sub">上传音频/视频文件，离线转写归档</p>
      </div>
    </div>

    <!-- 拖拽区 / 文件信息 -->
    <div
      class="dropzone"
      :class="{
        hover: dragHover,
        filled: phase === 'selected' || phase === 'transcribing' || phase === 'done' || phase === 'error',
      }"
      @click="phase === 'idle' || phase === 'error' ? pickFile() : undefined"
    >
      <template v-if="!pickedPath">
        <div class="dropzone-icon"><Icon name="attachment" :size="40" /></div>
        <p class="dropzone-text">拖入音频文件或点击选择</p>
        <p class="dropzone-hint">支持 {{ SUPPORTED_EXT.join(' / ') }}</p>
      </template>
      <template v-else>
        <div class="file-card">
          <div class="file-icon"><Icon name="file" :size="32" /></div>
          <div class="file-info">
            <p class="file-name">{{ pickedName }}</p>
            <p class="file-meta">
              <span v-if="sizeDisplay">{{ sizeDisplay }}</span>
              <span v-if="fileExt" class="file-ext">.{{ fileExt }}</span>
            </p>
          </div>
          <button
            v-if="phase === 'selected' || phase === 'error'"
            type="button"
            class="file-clear"
            @click.stop="clearPicked"
          >
            <Icon name="close" :size="16" />
          </button>
        </div>
      </template>
    </div>

    <!-- 控制区 -->
    <div class="upload-controls">
      <Dropdown
        :options="langOptions"
        :model-value="lang"
        placeholder="语言"
        size="sm"
        width="auto"
        :disabled="phase === 'transcribing'"
        @update:model-value="lang = String($event)"
      />
      <Button
        variant="primary"
        size="sm"
        :loading="phase === 'transcribing'"
        :disabled="!pickedPath || phase === 'transcribing'"
        @click="startTranscribe"
      >
        {{ phase === 'transcribing' ? '转写中' : '开始转写' }}
      </Button>
    </div>

    <!-- 进度条 -->
    <div v-if="phase === 'transcribing'" class="progress-area">
      <div class="progress-bar">
        <div class="progress-fill" :style="{ width: progressPct + '%' }" />
      </div>
      <p class="progress-text">
        <span>{{ progressPct }}%</span>
        <span v-if="progressStatus" class="progress-status">{{ progressStatus }}</span>
      </p>
    </div>

    <!-- 结果 -->
    <div v-if="(phase === 'done' || phase === 'error') && (result || errorMessage)" ref="resultRef" class="result-area">
      <div v-if="phase === 'error'" class="error-block">
        <Icon name="warning" :size="18" />
        <p>{{ errorMessage }}</p>
      </div>
      <template v-else-if="result">
        <div class="result-section">
          <div class="result-head">
            <Icon name="book" :size="16" />
            <span>转写文本</span>
          </div>
          <div class="result-transcript">{{ result.transcript }}</div>
        </div>
        <div v-if="result.summary" class="result-section">
          <div class="result-head summary-head">
            <Icon name="sparkles" :size="16" />
            <span>AI 摘要</span>
          </div>
          <div class="result-summary">{{ result.summary }}</div>
        </div>
        <div class="result-actions">
          <Button variant="primary" size="sm" @click="gotoHistory">
            查看历史记录
          </Button>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.asr-tab {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow-y: auto;
  background: var(--bg);
  padding: var(--space-6) var(--space-8);
  gap: var(--space-5);
}

.upload-head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-shrink: 0;
}

.upload-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  border-radius: var(--radius-lg);
  background: rgba(74, 158, 255, 0.12);
  color: var(--info);
  flex-shrink: 0;
}

.upload-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text);
}

.upload-sub {
  margin: 2px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
}

/* ---------- 拖拽区 ---------- */
.dropzone {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  min-height: 180px;
  padding: var(--space-6);
  border: 2px dashed var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  cursor: pointer;
  transition: border-color var(--duration-base) var(--ease-standard),
    background var(--duration-base) var(--ease-standard),
    transform var(--duration-base) var(--ease-standard);
  flex-shrink: 0;
}

.dropzone:hover:not(.filled) {
  border-color: var(--primary);
  background: var(--card-2);
}

.dropzone.hover {
  border-color: var(--primary);
  border-style: solid;
  background: rgba(74, 126, 255, 0.06);
  transform: scale(1.01);
}

.dropzone.filled {
  cursor: default;
  border-style: solid;
  padding: var(--space-4) var(--space-5);
}

.dropzone-icon {
  color: var(--muted);
  margin-bottom: var(--space-1);
}

.dropzone-text {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 500;
  color: var(--text);
}

.dropzone-hint {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
}

/* 已选文件卡片 */
.file-card {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
}

.file-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 48px;
  height: 48px;
  border-radius: var(--radius-lg);
  background: var(--card-2);
  color: var(--primary);
  flex-shrink: 0;
}

.file-info {
  flex: 1;
  min-width: 0;
}

.file-name {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 500;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-meta {
  margin: 2px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  display: flex;
  gap: var(--space-2);
}

.file-ext {
  text-transform: uppercase;
}

.file-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--duration-fast), color var(--duration-fast);
}

.file-clear:hover {
  background: var(--card-2);
  color: var(--danger);
}

/* ---------- 控制区 ---------- */
.upload-controls {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-shrink: 0;
}

/* ---------- 进度条 ---------- */
.progress-area {
  flex-shrink: 0;
}

.progress-bar {
  height: 6px;
  background: var(--card-2);
  border-radius: var(--radius-full);
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: var(--primary);
  border-radius: var(--radius-full);
  /* 平滑过渡而非跳跃：转写进度事件可能稀疏 */
  transition: width 0.4s var(--ease-emphasized);
}

.progress-text {
  display: flex;
  justify-content: space-between;
  margin: 6px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
}

.progress-status {
  color: var(--primary);
}

/* ---------- 结果区 ---------- */
.result-area {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  flex: 1;
  min-height: 0;
}

.error-block {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-lg);
  background: rgba(255, 92, 92, 0.08);
  border: 1px solid rgba(255, 92, 92, 0.3);
  color: var(--danger);
  font-size: var(--fs-sm);
}

.error-block p {
  margin: 0;
}

.result-section {
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--space-4) var(--space-5);
}

.result-head {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  margin-bottom: var(--space-2);
}

.result-head.summary-head {
  color: var(--primary);
}

.result-transcript,
.result-summary {
  font-size: var(--fs-base);
  line-height: 1.7;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 280px;
  overflow-y: auto;
}

.result-actions {
  display: flex;
  justify-content: flex-end;
}
</style>
