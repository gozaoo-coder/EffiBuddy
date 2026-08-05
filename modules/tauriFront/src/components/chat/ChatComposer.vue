<script setup lang="ts">
/**
 * ChatComposer —— 图一风格底部输入栏(卡片式复合输入框)
 *
 * 布局:圆角容器内上方为全宽 textarea,下方依次为
 * 「添加图片和文件」胶囊按钮(支持 Ctrl+U 快捷键)与底部操作栏
 * (+ 按钮 / meta pills / 右侧圆角方形发送按钮)。
 * 发送编排在 useChatSend(引用拼接 → 建会话 → 流式调用)实现,本组件只渲染 UI。
 */
import { ref, computed, inject, watch, onMounted, onUnmounted } from 'vue'
import { animate } from 'animejs'
import { Button, IconButton, Icon, Menu, type MenuItemOption } from '../basic'
import { CHAT_STORE_KEY } from '../../composables/chat/store'

const store = inject(CHAT_STORE_KEY)!

// 解构 ref:模板自动解包,script 中 .value 读写
const {
    input,
    sending,
    queuedCount,
    workingDir,
    thinking,
    reasoningEffort,
    workingDirSheetOpen,
    toolSheetOpen,
    shellBarExpanded,
    shellActiveCount,
    toggleShellBar,
  } = store.core
const { quoteChips, scrollToMessage, removeQuote } = store.menu
  const { compressBadgeInfo, compressSavedInfo, compressionSheetOpen } = store.compression
  const { versioning } = store
  const { sheetOpen: versionSheetOpen } = versioning
  // 发送编排已抽到 useChatSend(core/streaming/menu/autoscroll 组合),
  // 输入栏只保留 UI:渲染按钮状态 + 触发发送/停止。
  const { send, stopGenerating } = store.send

const composerFocused = ref(false)
const textareaRef = ref<HTMLTextAreaElement | null>(null)

// 发送后 input 被清空:回弹 textarea 高度到单行(useChatSend 不再关心 UI)
watch(input, (val, old) => {
  if (val === '' && old !== '') {
    const ta = textareaRef.value
    if (!ta) return
    ta.style.height = 'auto'
    const target = Math.min(ta.scrollHeight, 120)
    ta.style.height = target + 'px'
    void ta.offsetHeight
    animate(ta, {
      height: '56px',
      duration: 200,
      ease: 'out(3)',
    })
  }
})

// textarea 高度动画(关键:禁止 height: fit-content,用 animejs 动画)
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
  // 目标高度:不低于 56px(两行),不超过 120px
  const targetHeight = Math.min(Math.max(naturalHeight, 56), 120)
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

// ---------- 推理设置菜单（思考开关 + reasoning_effort 等级） ----------
const reasoningMenuVisible = ref(false)
const reasoningBtnRef = ref<HTMLElement | null>(null)

const effortLabels: Record<'low' | 'high' | 'max', string> = {
  low: '低',
  high: '高',
  max: '顶级',
}

/** pill 文案:关闭 → 「思考已关」;开启 → 「思考·等级」 */
const reasoningLabel = computed(() =>
  thinking.value ? `思考·${effortLabels[reasoningEffort.value]}` : '思考已关',
)

const reasoningItems = computed<MenuItemOption[]>(() => [
  { key: 'off', label: '关闭思考', selected: !thinking.value },
  {
    key: 'low',
    label: '低',
    selected: thinking.value && reasoningEffort.value === 'low',
    divided: true,
  },
  { key: 'high', label: '高', selected: thinking.value && reasoningEffort.value === 'high' },
  { key: 'max', label: '顶级', selected: thinking.value && reasoningEffort.value === 'max' },
])

function onReasoningSelect(item: MenuItemOption) {
  if (item.key === 'off') {
    thinking.value = false
    return
  }
  thinking.value = true
  reasoningEffort.value = item.key as 'low' | 'high' | 'max'
}

// ---------- Ctrl+U 全局快捷键:打开「添加图片和文件」工具 Sheet ----------
function onGlobalKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && (e.key === 'u' || e.key === 'U')) {
    e.preventDefault()
    toolSheetOpen.value = true
  }
}
onMounted(() => window.addEventListener('keydown', onGlobalKeydown))
onUnmounted(() => window.removeEventListener('keydown', onGlobalKeydown))

</script>

<template>
  <!-- 图一风格底部输入栏:卡片式复合输入框 -->
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

    <!-- composer-container 包裹层:上方输入区 + 中部附件胶囊 + 底部操作栏 -->
    <div class="composer-container">
      <textarea
        ref="textareaRef"
        v-model="input"
        class="composer-input"
        :placeholder="
          sending
            ? queuedCount > 0
              ? `生成中…可继续输入（已排队 ${queuedCount} 条，将插入下一轮）`
              : '生成中…可继续输入（将插入下一轮）'
            : '随便问点什么，Ctrl+U 可添加图片和文件…'
        "
        rows="2"
        @keydown="onKeydown"
        @focus="composerFocused = true"
        @blur="composerFocused = false"
        @input="autoResize"
      ></textarea>

      <!-- 「添加图片和文件」胶囊按钮(图一风格,Ctrl+U 快捷键) -->
      <div class="composer-attach-row">
        <button
          type="button"
          class="attach-pill"
          title="添加图片和文件（Ctrl+U）"
          @click="toolSheetOpen = true"
        >
          <Icon name="attachment" :size="14" />
          <span>添加图片和文件</span>
          <kbd class="attach-pill-kbd">Ctrl U</kbd>
        </button>
      </div>

      <!-- 底部操作栏:+ 按钮 + meta pills + 右侧发送按钮 -->
      <div class="composer-actions">
        <IconButton size="md" container title="附件" @click="toolSheetOpen = true">
          <Icon name="plus" :size="20" />
        </IconButton>
        <!-- 推理设置:点击弹出 Menu 选择思考开关与 reasoning_effort 等级 -->
        <button
          ref="reasoningBtnRef"
          type="button"
          class="meta-pill meta-pill--reasoning"
          :class="{ 'meta-pill--reasoning-on': thinking }"
          title="推理设置（思考开关 / 推理强度）"
          @click="reasoningMenuVisible = !reasoningMenuVisible"
        >
          <Icon name="thinking" :size="14" />
          <span class="meta-pill-text">{{ reasoningLabel }}</span>
          <Icon :name="reasoningMenuVisible ? 'chevron-down' : 'chevron-up'" :size="13" />
        </button>
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
          <!-- 生成中排队指示:AI 生成期间用户发送的消息将在下一轮插入 -->
          <button
            v-if="queuedCount > 0"
            type="button"
            class="meta-pill meta-pill--queued"
            :title="`${queuedCount} 条消息将在 AI 的下一个回复轮次前插入`"
          >
            <Icon name="clock" :size="14" />
            <span class="meta-pill-text">已排队 {{ queuedCount }} 条</span>
          </button>
          <!-- 压缩状态徽章:仅当当前会话已有压缩状态时显示,点击跳到压缩浮窗 -->
        <button
          v-if="compressBadgeInfo"
          type="button"
          class="meta-pill meta-pill--compress"
              :title="`当前会话已压缩 ${compressBadgeInfo.count} 条消息（第 ${compressBadgeInfo.level} 级 · ${compressBadgeInfo.actionCount} 条决策）${compressSavedInfo && compressSavedInfo.savedTokens > 0 ? ` · 节省约 ${compressSavedInfo.savedTokens} tokens` : ''} · 点击查看`"
              @click="compressionSheetOpen = true"
            >
              <Icon name="merge" :size="14" />
                <span class="meta-pill-text">已压缩 {{ compressBadgeInfo.count }}<template v-if="compressBadgeInfo.level > 0">·L{{ compressBadgeInfo.level }}</template><template v-if="compressSavedInfo && compressSavedInfo.savedTokens > 0">·↓{{ compressSavedInfo.savedTokens }}</template></span>
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
          <!-- 会话版本管理入口:分支 / 临时版本 / 回溯 / 撤回 -->
          <button
            type="button"
            class="meta-pill meta-pill--ver"
            title="会话版本管理（分支 / 临时版本 / 回溯 / 撤回）"
              @click="versionSheetOpen = true"
          >
            <Icon name="history" :size="14" />
              <span class="meta-pill-text">版本</span>
          </button>

          <!-- 右侧占位:把发送/停止按钮推到操作栏最右 -->
          <div class="composer-actions-spacer" />

          <!-- AI 生成中:右侧按钮变为红色「停止生成」(点击取消当前流) -->
          <Button
            v-if="sending"
            icon-only
            size="md"
            variant="danger"
            class="composer-send"
            title="停止生成"
            @click="stopGenerating"
          >
            <template #icon><Icon name="stop" :size="18" /></template>
          </Button>
          <Button
            v-else
            icon-only
            size="md"
            :variant="input.trim() ? 'primary' : 'normal'"
            :disabled="!input.trim()"
            class="composer-send"
            title="发送（Enter）"
            @click="send"
          >
            <template #icon><Icon name="arrow-up" :size="18" /></template>
          </Button>
        </div>
    </div>

    <!-- 推理设置菜单:位于输入栏上方弹出 -->
    <Menu
      v-model:visible="reasoningMenuVisible"
      :items="reasoningItems"
      :trigger-ref="reasoningBtnRef"
      title="推理设置"
      placement="top-start"
      :min-width="140"
      @select="onReasoningSelect"
    />
  </div>
</template>

<style scoped>
.composer-input {
  width: 100%;
  resize: none;
  min-height: 56px;
  max-height: 120px;
  padding: 6px 8px 4px;
  font-family: inherit;
  font-size: 15px;
  line-height: 1.5;
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

/* composer-container 包裹层:亮色浅灰,暗色用 --card-2 */
.composer-container {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px 8px;
  background: var(--card-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

[data-theme='light'] .composer-container {
  background: #f8f9fa;
}

.composer-kimi.focused .composer-container {
  border-color: color-mix(in srgb, var(--primary) 50%, var(--border));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 12%, transparent);
}

/* ---------- 「添加图片和文件」胶囊按钮 ---------- */
.composer-attach-row {
  display: flex;
  align-items: center;
  padding: 0 2px;
}

.attach-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  font-size: 12px;
  color: var(--muted);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: color 0.15s ease, background 0.15s ease, border-color 0.15s ease;
}

.attach-pill:hover {
  color: var(--text);
  border-color: color-mix(in srgb, var(--primary) 35%, var(--border));
  background: color-mix(in srgb, var(--primary) 6%, var(--card));
}

.attach-pill-kbd {
  padding: 1px 6px;
  font-size: 10px;
  font-family: inherit;
  font-variant-numeric: tabular-nums;
  color: var(--muted);
  background: var(--bg-2);
  border: 1px solid var(--border);
  border-radius: 4px;
}

/* ---------- 底部操作栏:+ 按钮 / meta pills / 右侧发送按钮 ---------- */
.composer-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  padding: 0 2px;
}

.composer-actions-spacer {
  flex: 1;
  min-width: 8px;
}

/* 右侧发送/停止按钮:圆角方形,与操作栏高度对齐 */
.composer-send {
  flex-shrink: 0;
}

/* 推理设置 pill:开启思考时用 primary 收敛色高亮 */
.meta-pill--reasoning-on {
  color: var(--primary);
}

.meta-pill--reasoning-on:hover,
.meta-pill--reasoning:hover {
  color: var(--primary);
  border-color: color-mix(in srgb, var(--primary) 30%, var(--border));
  background: color-mix(in srgb, var(--primary) 8%, transparent);
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

  /* 生成中排队指示:AI 生成期间用户发送的消息将在下一轮插入 */
  .meta-pill--queued {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 30%, var(--border));
    background: color-mix(in srgb, var(--warn) 10%, transparent);
  }

  .meta-pill--queued:hover {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, var(--border));
    background: color-mix(in srgb, var(--warn) 14%, transparent);
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

  /* 会话版本管理入口:primary 收敛色,与面板开关同风格 */
  .meta-pill--ver:hover {
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
