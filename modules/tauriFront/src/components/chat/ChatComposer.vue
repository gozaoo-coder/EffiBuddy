<script setup lang="ts">
/**
 * ChatComposer —— Kimi 风格底部输入栏
 *
 * 内聚:引用块 chips、textarea(Enter 发送 / Shift+Enter 换行 / 高度动画)、
 * 发送/语音按钮、meta pills(工作区 / 压缩徽章 / 右栏面板开关)。
 * send() 编排(引用拼接 → 建会话 → 流式调用)在此实现。
 */
import { ref, inject, nextTick } from 'vue'
import { animate } from 'animejs'
import { invoke } from '@tauri-apps/api/core'
import { Button, IconButton, Icon, useToast } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!
const { toast } = useToast()

// 解构 ref:模板自动解包,script 中 .value 读写
const {
  input,
  sending,
  workingDir,
  workingDirSheetOpen,
  toolSheetOpen,
  ctxPanelOpen,
  toggleCtxPanel,
  shellBarExpanded,
  shellActiveCount,
  toggleShellBar,
  ensureConversation,
  newId,
} = store.core
const { quoteChips, scrollToMessage, removeQuote, buildQuoteContext, clearQuotes } = store.menu
const { addMessage } = store.streaming
const { stickToBottom } = store.autoscroll
const { beginNewTurn } = store.taskMode
const { compressBadgeInfo, compressionSheetOpen } = store.compression

const composerFocused = ref(false)
const textareaRef = ref<HTMLTextAreaElement | null>(null)

// composer-inner 高度动画(关键:禁止 height: fit-content,用 animejs 动画)
function autoResize() {
  const ta = textareaRef.value
  if (!ta) return
  // 当前高度(animejs 动画起点)
  const currentHeight = ta.offsetHeight
  // 临时设为 auto 测量自然内容高度(同步操作,不触发重绘)
  ta.style.height = 'auto'
  const naturalHeight = ta.scrollHeight
  // 立即恢复当前高度,避免视觉跳变
  ta.style.height = currentHeight + 'px'
  // 目标高度:不超过 120px
  const targetHeight = Math.min(naturalHeight, 120)
  // 强制 reflow,确保 animejs 起点正确
  void ta.offsetHeight
  animate(ta, {
    height: targetHeight + 'px',
    duration: 200,
    ease: 'out(3)',
  })
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault()
    void send()
  }
}

// ---------- 发送(流式) ----------
async function send() {
  const content = input.value.trim()
  if (!content || sending.value) return

  // 拼接引用上下文到 content 前面(引用前缀仅发给后端,用户气泡展示纯 content)
  const finalContent = buildQuoteContext(content)

  // 用户主动发送:强制跟随到底部
  stickToBottom.value = true

  // 没有当前会话时新建一个(新建对话页签:id 为 null 或 __new_chat__ 哨兵)
  const id = await ensureConversation()
  if (!id) return

  sending.value = true
  // 新一轮用户输入:重置「是否任务回合」标记(新内容不再合并进旧长程任务气泡)
  beginNewTurn()
  input.value = ''
  clearQuotes()
  // 重置 textarea 高度(JS 赋值不触发 input 事件,需手动动画回 40px)
  await nextTick()
  if (textareaRef.value) {
    animate(textareaRef.value, {
      height: [textareaRef.value.offsetHeight + 'px', '40px'],
      duration: 200,
      ease: 'out(3)',
    })
  }

  // 用户气泡展示纯 content(不含引用前缀)
  await addMessage({
    id: newId(),
    role: 'user',
    content,
    timestamp: Date.now(),
  })

  try {
    await invoke('send_message_stream', {
      conversationId: id,
      content: finalContent,
    })
  } catch (e) {
    sending.value = false
    await addMessage({
      id: newId(),
      role: 'system',
      content: `请求失败：${e}`,
      timestamp: Date.now(),
    })
    toast({ content: `请求失败：${e}`, type: 'error' })
  }
}
</script>

<template>
  <!-- Kimi 风格底部输入栏 -->
  <div class="composer-kimi" :class="{ focused: composerFocused }">
    <!-- 引用块区 -->
    <div v-if="quoteChips.length" class="quote-chips">
      <div
        v-for="q in quoteChips"
        :key="q.messageId"
        class="quote-chip"
        @click="scrollToMessage(q.messageId)"
      >
        <Icon name="quote" :size="14" />
        <span class="quote-chip-text">{{ q.snippet }}</span>
        <button
          type="button"
          class="quote-chip-close"
          title="移除引用"
          @click.stop="removeQuote(q.messageId)"
        >
          <Icon name="close" :size="14" />
        </button>
      </div>
    </div>

    <!-- composer-container 包裹层 -->
    <div class="composer-container">
      <div class="composer-inner">
        <IconButton size="md" container title="附件" @click="toolSheetOpen = true">
          <Icon name="plus" :size="22" />
        </IconButton>
        <textarea
          ref="textareaRef"
          v-model="input"
          class="composer-input"
          :placeholder="sending ? '生成中…' : '尽管问，带图也行'"
          :disabled="sending"
          rows="1"
          @keydown="onKeydown"
          @focus="composerFocused = true"
          @blur="composerFocused = false"
          @input="autoResize"
        ></textarea>
        <Button
          v-if="!input.trim()"
          icon-only
          shape="circle"
          size="md"
          variant="normal"
          title="语音输入"
          @click="toast({ content: '语音输入即将上线', type: 'info' })"
        >
          <template #icon><Icon name="mic" :size="22" /></template>
        </Button>
        <Button
          v-else
          icon-only
          shape="circle"
          size="md"
          variant="primary"
          :disabled="!input.trim()"
          title="发送"
          @click="send"
        >
          <template #icon><Icon name="arrow-up" :size="22" /></template>
        </Button>
      </div>
      <!-- 工作区 + 压缩 + 面板开关(输出栏圆环+token 显示已移除)-->
      <div class="composer-meta">
        <button
          type="button"
          class="meta-pill meta-pill--wd"
          :title="workingDir ?? '未设置'"
          @click="workingDirSheetOpen = true"
        >
          <Icon name="folder" :size="14" />
          <span class="meta-pill-text meta-pill-text--ellipsis">
            {{ workingDir ? workingDir : '默认工作区' }}
          </span>
        </button>
        <!-- 压缩状态徽章:仅当当前会话已有压缩状态时显示,点击跳到压缩浮窗 -->
        <button
          v-if="compressBadgeInfo"
          type="button"
          class="meta-pill meta-pill--compress"
          :title="`当前会话已压缩 ${compressBadgeInfo.count} 条消息（${compressBadgeInfo.actionCount} 条决策）· 点击查看`"
          @click="compressionSheetOpen = true"
        >
          <Icon name="merge" :size="14" />
          <span class="meta-pill-text">已压缩 {{ compressBadgeInfo.count }}</span>
        </button>
        <!-- 命令会话折叠开关:展开/收起底部 ShellSessionBar(实时展示 AI 的 shell 工作状态) -->
        <button
          type="button"
          class="meta-pill meta-pill--ss"
          :class="{ 'meta-pill--ss-on': shellBarExpanded }"
          :title="shellBarExpanded ? '折叠命令会话栏' : '展开命令会话栏'"
          @click="toggleShellBar()"
        >
          <Icon name="keyboard" :size="14" />
          <span class="meta-pill-text">
            命令会话
            <span v-if="shellActiveCount > 0" class="meta-pill-badge">{{ shellActiveCount }}</span>
          </span>
          <Icon :name="shellBarExpanded ? 'chevron-down' : 'chevron-up'" :size="13" />
        </button>
        <!-- 右栏上下文面板开关 -->
        <button
          type="button"
          class="meta-pill meta-pill--ctx"
          :class="{ 'meta-pill--ctx-on': ctxPanelOpen }"
          :title="ctxPanelOpen ? '收起上下文面板' : '展开上下文面板（todoTree / 用量 / 压缩）'"
          @click="toggleCtxPanel()"
        >
          <Icon name="discover" :size="14" />
          <span class="meta-pill-text">{{ ctxPanelOpen ? '收起面板' : '展开面板' }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.composer-input {
  flex: 1;
  resize: none;
  min-height: 40px;
  max-height: 120px;
  padding: 10px 12px;
  font-family: inherit;
  font-size: 15px;
  color: var(--text);
  background: transparent;
  border: none;
  outline: none;
}

.composer-input::placeholder {
  color: var(--muted);
}

.composer-input:disabled {
  opacity: 0.7;
  cursor: not-allowed;
}

/* ---------- 引用块 ---------- */
.quote-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: 0 4px 8px;
}

.quote-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 280px;
  padding: 4px 8px 4px 10px;
  font-size: 12px;
  color: var(--text);
  background: color-mix(in srgb, var(--primary) 8%, var(--card));
  border: 1px solid color-mix(in srgb, var(--primary) 24%, var(--border));
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease;
}

.quote-chip:hover {
  background: color-mix(in srgb, var(--primary) 14%, var(--card));
  border-color: var(--primary);
}

.quote-chip-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.quote-chip-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  flex-shrink: 0;
}

.quote-chip-close:hover {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

/* ---------- composer 升级 ---------- */
/* focus 上抬:用 transform 避免 layout reflow,配合 transition 平滑 */
.composer-kimi {
  transition: transform 0.18s ease;
}

.composer-kimi.focused {
  transform: translateY(-2px);
}

/* composer-container 包裹层:亮色 #CFCFCF,暗色用 --card-2 */
.composer-container {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

[data-theme='light'] .composer-container {
  background: #eeeeee;
}

.composer-kimi.focused .composer-container {
  border-color: color-mix(in srgb, var(--primary) 50%, var(--border));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 12%, transparent);
}

/* composer-inner 高度跟随 textarea;overflow hidden 防止超出时溢出 */
.composer-inner {
  display: flex;
  align-items: flex-end;
  gap: 6px;
}

/* 上下文 ring + 工作区 meta 行 */
.composer-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 2px;
  flex-wrap: wrap;
}

.meta-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  color: var(--muted);
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: color 0.15s ease, background 0.15s ease, border-color 0.15s ease;
}

.meta-pill:hover {
  color: var(--text);
  background: var(--bg-2);
  border-color: var(--border);
}

.meta-pill--wd {
  max-width: 240px;
}

/* 压缩状态徽章:仅当会话已压缩时显示,配色用 success 收敛色 */
.meta-pill--compress {
  color: var(--success);
}

.meta-pill--compress:hover {
  color: var(--success);
  border-color: color-mix(in srgb, var(--success) 30%, var(--border));
  background: color-mix(in srgb, var(--success) 8%, transparent);
}

[data-theme='light'] .meta-pill--compress {
  background: rgba(16, 163, 127, 0.08);
}

/* 命令会话折叠开关:激活(展开)态用 primary 收敛色,徽标显示运行中数量 */
.meta-pill--ss:hover,
.meta-pill--ss-on {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill--ss-on:hover {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 5px;
  border-radius: var(--radius-full);
  font-size: 10px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: var(--success);
  background: color-mix(in srgb, var(--success) 14%, transparent);
}

.meta-pill--ctx {
  margin-left: auto;
}

.meta-pill--ctx:hover {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill--ctx-on {
  color: var(--primary);
}

.meta-pill--ctx-on:hover {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.meta-pill-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta-pill-text--ellipsis {
  max-width: 200px;
}
</style>
