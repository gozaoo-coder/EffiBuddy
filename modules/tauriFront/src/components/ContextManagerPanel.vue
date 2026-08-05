<script setup lang="ts">
/**
 * ContextManagerPanel 上下文管理统一入口
 *
 * 作为 SettingsPanel 的 "context" tab 内容嵌入（不自带 BindSheet 容器）。
 * 三个 sub-tab 统一展示当前 agent 注入到 LLM 的完整上下文：
 * - 永久记忆：始终注入的偏好/事实/指令（复用 PinnedMemoryPanel 组件）
 * - 系统提示词：编辑当前 agent 的 preamble（通过 set_config 持久化并热替换）
 * - 上下文预览：实时展示当前会话将拼装的 prompt 结构（按段可视化）
 *
 * 设计原则：
 * - 不重复 PinnedMemoryPanel 已有的 CRUD 逻辑，直接嵌入子组件
 * - preamble 编辑实时显示 dirty 状态，避免误以为未保存
 * - 上下文预览明确展示各段的"是否激活"与"内容长度"，让用户直观理解 prompt 拼装
 */
import { ref, computed, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Button,
  Icon,
  SegmentedButton,
  IconButton,
  useToast,
  type SegmentedOption,
} from './basic'
import PinnedMemoryPanel from './PinnedMemoryPanel.vue'
import { useAnimeTransition } from '../composables/useAnimeTransition'
import type { AgentConfig, ContextPreview } from '../types'

const props = defineProps<{
  /** 面板是否打开（透传给 PinnedMemoryPanel） */
  open: boolean
  /** 当前会话 id（用于上下文预览）；为 null 时仅展示 preamble + 永久记忆 */
  conversationId: string | null
}>()

const { toast } = useToast()

// ---------- sub-tab 切换 ----------
type SubTab = 'pinned' | 'preamble' | 'preview'
const activeTab = ref<SubTab>('pinned')

const subTabs: SegmentedOption[] = [
  { label: '永久记忆', value: 'pinned' },
  { label: '系统提示词', value: 'preamble' },
  { label: '上下文预览', value: 'preview' },
]

// sub-tab 切换淡入动画
const { onEnter, onLeave } = useAnimeTransition({
  enter: {
    opacity: [0, 1],
    translateY: [10, 0],
    duration: 300,
    ease: 'out(3)',
  },
  leave: {
    opacity: [1, 0],
    translateY: [0, -8],
    duration: 200,
    ease: 'inOut(2)',
  },
})

// ---------- 系统提示词 ----------
const config = ref<AgentConfig | null>(null)
const preambleDraft = ref('')
const preambleSaving = ref(false)

const preambleDirty = computed(
  () => config.value !== null && preambleDraft.value !== config.value.preamble,
)

async function loadConfig() {
  try {
    config.value = await invoke<AgentConfig>('get_config')
    preambleDraft.value = config.value.preamble
  } catch (e) {
    toast({ content: `加载配置失败：${e}`, type: 'error' })
  }
}

async function savePreamble() {
  if (!config.value || !preambleDirty.value) return
  preambleSaving.value = true
  try {
    const newConfig: AgentConfig = { ...config.value, preamble: preambleDraft.value }
    await invoke('set_config', { config: newConfig })
    config.value = newConfig
    toast({ content: '系统提示词已保存并热替换 agent', type: 'success' })
    // 同步刷新上下文预览
    if (activeTab.value === 'preview') loadPreview()
  } catch (e) {
    toast({ content: `保存失败：${e}`, type: 'error' })
  } finally {
    preambleSaving.value = false
  }
}

function resetPreamble() {
  if (config.value) preambleDraft.value = config.value.preamble
}

// ---------- 上下文预览 ----------
const preview = ref<ContextPreview | null>(null)
const previewLoading = ref(false)
const previewError = ref(false)

async function loadPreview() {
  previewLoading.value = true
  previewError.value = false
  try {
    preview.value = await invoke<ContextPreview | null>('get_context_preview', {
      conversationId: props.conversationId,
    })
  } catch (e) {
    toast({ content: `加载预览失败：${e}`, type: 'error' })
    preview.value = null
    previewError.value = true
  } finally {
    previewLoading.value = false
  }
}

function refreshPreview() {
  loadPreview()
}

// ---------- 按需加载 ----------
let configLoaded = false
let previewLoaded = false

watch(
  () => props.open,
  (v) => {
    if (v) {
      // 打开时如果当前 sub-tab 是 preamble/preview，加载对应数据
      if (activeTab.value === 'preamble' && !configLoaded) {
        configLoaded = true
        loadConfig()
      }
      if (activeTab.value === 'preview' && !previewLoaded) {
        previewLoaded = true
        loadPreview()
      }
    }
  },
)

watch(activeTab, (v) => {
  if (v === 'preamble' && !configLoaded) {
    configLoaded = true
    loadConfig()
  }
  if (v === 'preview') {
    // 每次切换到预览都重新加载（消息历史可能变化）
    loadPreview()
  }
})

// ---------- 预览面板辅助 ----------
const fullPromptChars = computed(() =>
  preview.value ? preview.value.full_prompt.length : 0,
)

const pinnedChars = computed(() =>
  preview.value ? preview.value.pinned_section.length : 0,
)

const memoryChars = computed(() =>
  preview.value ? preview.value.memory_section.length : 0,
)

const historyChars = computed(() =>
  preview.value ? preview.value.history_section.length : 0,
)

const currentQuestionChars = computed(() =>
  preview.value ? preview.value.current_question.length : 0,
)

const preambleChars = computed(() =>
  preview.value ? preview.value.preamble.length : 0,
)

// 当前会话状态
const hasActiveConversation = computed(() => !!props.conversationId)

// 复制完整 prompt 到剪贴板
async function copyFullPrompt() {
  if (!preview.value) return
  try {
    await navigator.clipboard.writeText(preview.value.full_prompt)
    toast({ content: '完整 prompt 已复制到剪贴板', type: 'success' })
  } catch (e) {
    toast({ content: `复制失败：${e}`, type: 'error' })
  }
}
</script>

<template>
  <section class="ctx-page">
    <header class="page-head">
      <div class="page-head-main">
        <h2 class="page-title">上下文管理</h2>
        <p class="page-sub">
          统一查看与编辑注入到 LLM 的所有上下文：永久记忆、系统提示词、当前对话历史
        </p>
      </div>
    </header>

    <!-- 当前会话状态条 -->
    <div class="conv-status" :class="{ 'conv-status--active': hasActiveConversation }">
      <span class="conv-status-icon">
        <Icon :name="hasActiveConversation ? 'chat' : 'info'" :size="16" />
      </span>
      <span class="conv-status-text">
        {{ hasActiveConversation ? `当前会话已选中（用于上下文预览）` : '未选中会话（上下文预览仅展示 preamble 与永久记忆）' }}
      </span>
      <button
        v-if="activeTab === 'preview'"
        type="button"
        class="conv-status-action"
        @click="refreshPreview"
      >
        <Icon name="refresh" :size="14" />
        刷新
      </button>
    </div>

    <!-- sub-tab 切换 -->
    <div class="sub-tabs">
      <SegmentedButton
        v-model="activeTab"
        :options="subTabs"
        size="md"
        block
      />
    </div>

    <!-- sub-tab 内容 -->
    <Transition :css="false" @enter="onEnter" @leave="onLeave" mode="out-in">
      <!-- 永久记忆 sub-tab：直接复用 PinnedMemoryPanel -->
      <PinnedMemoryPanel
        v-if="activeTab === 'pinned'"
        key="pinned"
        :open="props.open"
      />

      <!-- 系统提示词 sub-tab -->
      <section v-else-if="activeTab === 'preamble'" key="preamble" class="sub-page">
        <div class="card preamble-card">
          <div class="card-head">
            <div class="card-head-text">
              <span class="card-title">当前 agent 的系统提示词</span>
              <span class="card-hint">
                作为 system 消息注入到每轮对话开头，定义 agent 人设与行为约束
              </span>
            </div>
            <span class="char-badge">{{ preambleDraft.length }} 字符</span>
          </div>

          <textarea
            v-model="preambleDraft"
            rows="6"
            class="preamble-textarea"
            placeholder="例如：你是 EffiSuite 的 AI 助手。简短回答；调用工具前先用 1-3 句话简述最优实现路径，再发起调用。"
          ></textarea>

          <div class="preamble-actions">
            <span class="dirty-hint" :class="{ 'dirty-hint--active': preambleDirty }">
              <Icon :name="preambleDirty ? 'edit' : 'check'" :size="12" />
              {{ preambleDirty ? '未保存' : '已同步' }}
            </span>
            <div class="preamble-actions-right">
              <Button
                variant="normal"
                size="sm"
                :disabled="!preambleDirty || preambleSaving"
                @click="resetPreamble"
              >
                重置
              </Button>
              <Button
                variant="primary"
                size="sm"
                :loading="preambleSaving"
                :disabled="!preambleDirty"
                @click="savePreamble"
              >
                保存并热替换
              </Button>
            </div>
          </div>
        </div>

        <div class="hint-card">
          <div class="hint-mark"><Icon name="info" :size="18" /></div>
          <div class="hint-text">
            <div class="hint-title">关于模型级 preamble</div>
            <p class="hint-desc">
              此处编辑的是当前激活模型的全局 preamble。
              每个模型在「可用模型配置」面板中也可单独设置 preamble，
              切换模型时会用该模型的 preamble 覆盖此处的值。
            </p>
          </div>
        </div>
      </section>

      <!-- 上下文预览 sub-tab -->
      <section v-else key="preview" class="sub-page">
        <!-- 加载中 -->
        <div v-if="previewLoading" class="preview-loading">
          <Icon name="refresh" :size="28" />
          <span>正在加载上下文预览...</span>
        </div>

        <!-- 后端不支持（MockAgent 或出错） -->
        <div v-else-if="!preview && previewError" class="preview-empty">
          <div class="empty-illust"><Icon name="info" :size="36" /></div>
          <p class="empty-text">无法加载上下文预览</p>
          <p class="empty-hint">可能是当前后端为 MockAgent 或加载失败，请刷新重试</p>
        </div>

        <!-- MockAgent 后端：返回 null -->
        <div v-else-if="!preview" class="preview-empty">
          <div class="empty-illust"><Icon name="info" :size="36" /></div>
          <p class="empty-text">当前后端不支持上下文预览</p>
          <p class="empty-hint">MockAgent 不构建 prompt，请在「可用模型配置」中配置真实模型</p>
        </div>

        <!-- 正常预览 -->
        <template v-else>
          <!-- 配置概览 -->
          <div class="stats-grid">
            <div class="stat-card">
              <span class="stat-label">永久记忆</span>
              <span class="stat-value">{{ preview.pinned_count }}</span>
              <span class="stat-unit">条</span>
            </div>
            <div class="stat-card" :class="{ 'stat-card--muted': !preview.memory_enabled }">
              <span class="stat-label">RAG 命中</span>
              <span class="stat-value">{{ preview.memory_hits_count }}</span>
              <span class="stat-unit">/ {{ preview.memory_inject_limit }}</span>
            </div>
            <div class="stat-card">
              <span class="stat-label">对话历史</span>
              <span class="stat-value">{{ preview.history_keep_count }}</span>
              <span class="stat-unit">/ {{ preview.history_total_count }}</span>
            </div>
            <div class="stat-card">
              <span class="stat-label">完整 prompt</span>
              <span class="stat-value">{{ fullPromptChars }}</span>
              <span class="stat-unit">字符</span>
            </div>
          </div>

          <!-- 拼装顺序说明 -->
          <div class="stack-card">
            <div class="stack-head">
              <span class="stack-title">注入顺序</span>
              <span class="stack-hint">从上到下拼装为最终 prompt 发给 LLM</span>
            </div>
            <div class="stack-flow">
              <div class="stack-node" :class="`stack-node--${preview.preamble ? 'active' : 'empty'}`">
                <span class="stack-node-label">① System 消息</span>
                <span class="stack-node-meta">{{ preambleChars }} 字符</span>
              </div>
              <div class="stack-connector"></div>
              <div class="stack-node" :class="`stack-node--${preview.pinned_section ? 'active' : 'empty'}`">
                <span class="stack-node-label">② [永久记忆] 段</span>
                <span class="stack-node-meta">{{ preview.pinned_count }} 条 · {{ pinnedChars }} 字符</span>
              </div>
              <div class="stack-connector"></div>
              <div class="stack-node" :class="`stack-node--${preview.memory_section ? 'active' : 'empty'}`">
                <span class="stack-node-label">③ [相关历史记忆] 段</span>
                <span class="stack-node-meta">{{ preview.memory_hits_count }} 条 · {{ memoryChars }} 字符</span>
              </div>
              <div class="stack-connector"></div>
              <div class="stack-node" :class="`stack-node--${preview.history_section ? 'active' : 'empty'}`">
                <span class="stack-node-label">④ [当前对话最近] 段</span>
                <span class="stack-node-meta">{{ preview.history_keep_count }}/{{ preview.history_total_count }} 条 · {{ historyChars }} 字符</span>
              </div>
              <div class="stack-connector"></div>
              <div class="stack-node stack-node--active">
                <span class="stack-node-label">⑤ [当前问题] 段</span>
                <span class="stack-node-meta">{{ currentQuestionChars }} 字符</span>
              </div>
            </div>
          </div>

          <!-- 各段详细内容 -->
          <div class="section-card">
            <div class="section-card-head">
              <span class="section-card-title">
                <Icon name="spark" :size="16" />
                ① 系统提示词（preamble）
              </span>
              <span class="section-card-badge" :class="`badge--${preview.preamble ? 'active' : 'empty'}`">
                {{ preview.preamble ? '已注入' : '空' }}
              </span>
            </div>
            <pre v-if="preview.preamble" class="section-card-body">{{ preview.preamble }}</pre>
            <p v-else class="section-card-empty">未设置 preamble</p>
          </div>

          <div class="section-card">
            <div class="section-card-head">
              <span class="section-card-title">
                <Icon name="pin" :size="16" />
                ② [永久记忆] 段
              </span>
              <span class="section-card-badge" :class="`badge--${preview.pinned_section ? 'active' : 'empty'}`">
                {{ preview.pinned_count }} 条
              </span>
            </div>
            <pre v-if="preview.pinned_section" class="section-card-body">{{ preview.pinned_section }}</pre>
            <p v-else class="section-card-empty">
              无永久记忆。可在「永久记忆」sub-tab 添加，或对话中说"请记住..."
            </p>
          </div>

          <div class="section-card">
            <div class="section-card-head">
              <span class="section-card-title">
                <Icon name="search" :size="16" />
                ③ [相关历史记忆] 段（RAG 自动检索）
              </span>
              <span class="section-card-badge" :class="`badge--${preview.memory_section ? 'active' : 'empty'}`">
                {{ preview.memory_enabled ? `${preview.memory_hits_count} / ${preview.memory_inject_limit} 条` : '未启用' }}
              </span>
            </div>
            <pre v-if="preview.memory_section" class="section-card-body">{{ preview.memory_section }}</pre>
            <p v-else class="section-card-empty">
              {{ preview.memory_enabled ? '当前问题未检索到相关跨会话记忆' : '未启用 RAG 记忆增强' }}
            </p>
          </div>

          <div class="section-card">
            <div class="section-card-head">
              <span class="section-card-title">
                <Icon name="chat" :size="16" />
                ④ [当前对话最近] 段
              </span>
              <span class="section-card-badge" :class="`badge--${preview.history_section ? 'active' : 'empty'}`">
                {{ preview.history_keep_count }} / {{ preview.history_total_count }} 条
              </span>
            </div>
            <pre v-if="preview.history_section" class="section-card-body">{{ preview.history_section }}</pre>
            <p v-else class="section-card-empty">
              {{ hasActiveConversation ? '当前会话暂无历史消息' : '未选中会话' }}
            </p>
            <p v-if="preview.memory_enabled" class="section-card-hint">
              启用 RAG 时仅保留最近 {{ preview.recent_history_limit }} 条，
              单条超过 {{ preview.history_truncate_chars }} 字符会被截断
            </p>
          </div>

          <div class="section-card">
            <div class="section-card-head">
              <span class="section-card-title">
                <Icon name="edit" :size="16" />
                ⑤ [当前问题] 段
              </span>
              <span class="section-card-badge badge--active">{{ currentQuestionChars }} 字符</span>
            </div>
            <pre v-if="preview.current_question" class="section-card-body">{{ preview.current_question }}</pre>
            <p v-else class="section-card-empty">
              {{ hasActiveConversation ? '当前会话没有用户消息' : '未选中会话' }}
            </p>
          </div>

          <!-- 完整 prompt -->
          <div class="section-card section-card--full">
            <div class="section-card-head">
              <span class="section-card-title">
                <Icon name="book" :size="16" />
                完整 prompt（拼装后实际发给 LLM）
              </span>
              <IconButton
                size="sm"
                title="复制完整 prompt"
                @click="copyFullPrompt"
              >
                <Icon name="edit" :size="16" />
              </IconButton>
            </div>
            <pre v-if="preview.full_prompt" class="section-card-body section-card-body--full">{{ preview.full_prompt }}</pre>
            <p v-else class="section-card-empty">空 prompt</p>
          </div>
        </template>
      </section>
    </Transition>
  </section>
</template>

<style scoped>
.ctx-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* ---------- 页头 ---------- */
.page-head {
  margin-bottom: 4px;
}

.page-head-main {
  min-width: 0;
}

.page-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text);
}

.page-sub {
  margin: 4px 0 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 当前会话状态条 ---------- */
.conv-status {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--card-2);
  font-size: var(--fs-sm);
  color: var(--muted);
}

.conv-status--active {
  border-color: var(--primary);
  background: rgba(74, 126, 255, 0.06);
  color: var(--primary);
}

.conv-status-icon {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.conv-status-text {
  flex: 1;
  min-width: 0;
}

.conv-status-action {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.conv-status-action:hover {
  border-color: var(--primary);
  color: var(--primary);
}

/* ---------- sub-tab 切换 ---------- */
.sub-tabs {
  margin: 4px 0;
}

/* ---------- sub-page ---------- */
.sub-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* ---------- 通用卡片 ---------- */
.card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.card-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.card-head-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.card-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.card-hint {
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

.char-badge {
  flex-shrink: 0;
  padding: 2px 10px;
  font-size: var(--fs-xs);
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

/* ---------- preamble 编辑区 ---------- */
.preamble-textarea {
  width: 100%;
  min-height: 140px;
  padding: 10px 12px;
  font-family: inherit;
  font-size: var(--fs-base);
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  outline: none;
  resize: vertical;
  line-height: 1.5;
  box-sizing: border-box;
  transition: border-color var(--duration-fast) var(--ease-standard);
}

.preamble-textarea:focus {
  border-color: var(--primary);
}

.preamble-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.dirty-hint {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--fs-xs);
  color: var(--muted);
}

.dirty-hint--active {
  color: var(--warn);
}

.preamble-actions-right {
  display: flex;
  gap: 8px;
}

/* ---------- 提示卡片 ---------- */
.hint-card {
  display: flex;
  gap: 12px;
  padding: 14px 16px;
  background: linear-gradient(135deg, rgba(74, 126, 255, 0.08), rgba(74, 126, 255, 0.02));
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.hint-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-md);
  background: var(--card-2);
  color: var(--primary);
  flex-shrink: 0;
}

.hint-text {
  min-width: 0;
  flex: 1;
}

.hint-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
  margin-bottom: 4px;
}

.hint-desc {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  line-height: 1.55;
}

/* ---------- 预览面板 ---------- */
.preview-loading,
.preview-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 40px 20px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
  background: var(--card);
  color: var(--muted);
}

.preview-loading {
  color: var(--primary);
}

.preview-loading .icon-spin {
  animation: spin 1.2s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.empty-illust {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background: var(--card-2);
  color: var(--muted);
}

.empty-text {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.empty-hint {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--muted);
  text-align: center;
  line-height: 1.5;
  max-width: 360px;
}

/* ---------- 统计网格 ---------- */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 10px;
}

.stat-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  text-align: center;
}

.stat-card--muted {
  opacity: 0.6;
}

.stat-label {
  font-size: var(--fs-xs);
  color: var(--muted);
}

.stat-value {
  font-size: var(--fs-xl);
  font-weight: 700;
  color: var(--text);
  font-family: 'SFMono-Regular', Consolas, monospace;
  line-height: 1.2;
}

.stat-unit {
  font-size: var(--fs-xs);
  color: var(--muted);
}

/* ---------- 注入顺序可视化 ---------- */
.stack-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.stack-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.stack-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text);
}

.stack-hint {
  font-size: var(--fs-xs);
  color: var(--muted);
}

.stack-flow {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 0;
}

.stack-node {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-2);
  font-size: var(--fs-sm);
}

.stack-node--active {
  border-color: var(--primary);
  background: rgba(74, 126, 255, 0.08);
}

.stack-node--empty {
  opacity: 0.55;
}

.stack-node-label {
  font-weight: 500;
  color: var(--text);
}

.stack-node-meta {
  font-size: var(--fs-xs);
  color: var(--muted);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.stack-connector {
  width: 2px;
  height: 12px;
  margin: 0 auto;
  background: var(--border);
}

/* ---------- 各段详细卡片 ---------- */
.section-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px 16px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.section-card--full {
  background: var(--card-2);
}

.section-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.section-card-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text);
}

.section-card-badge {
  flex-shrink: 0;
  padding: 2px 8px;
  font-size: var(--fs-xs);
  border-radius: var(--radius-full);
  font-family: 'SFMono-Regular', Consolas, monospace;
}

.badge--active {
  color: #fff;
  background: var(--primary);
}

.badge--empty {
  color: var(--muted);
  background: var(--card-2);
  border: 1px solid var(--border);
}

.section-card-body {
  margin: 0;
  padding: 10px 12px;
  font-family: 'SFMono-Regular', Consolas, 'JetBrains Mono', monospace;
  font-size: var(--fs-xs);
  line-height: 1.55;
  color: var(--text);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 280px;
  overflow-y: auto;
}

.section-card-body--full {
  max-height: 360px;
  background: var(--bg-2);
}

.section-card-empty {
  margin: 0;
  padding: 10px 12px;
  font-size: var(--fs-sm);
  color: var(--muted);
  background: var(--bg-2);
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  line-height: 1.5;
}

.section-card-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--muted);
  line-height: 1.5;
}

/* ---------- 响应式 ---------- */
@media (max-width: 540px) {
  .stats-grid {
    grid-template-columns: repeat(2, 1fr);
  }
}
</style>
