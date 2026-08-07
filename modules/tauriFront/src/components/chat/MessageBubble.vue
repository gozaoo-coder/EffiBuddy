<script setup lang="ts">
/**
 * MessageBubble —— 单条消息气泡
 *
 * 负责:气泡外壳(角色样式 / 流式标记)、长按/右键菜单交互、
 * 附件图片渲染(点击打开全屏预览)、计费统计悬浮明细。
 * 助手内容(推理/工具/子 agent/Markdown)委托 AssistantContent。
 */
import { inject } from 'vue'
import { Icon } from '../basic'
import AssistantContent from './AssistantContent.vue'
import { CHAT_STORE_KEY } from '../../composables/chat/store'
import type { Message } from '../../types'
import type { BubbleMeta } from '../../composables/chat/useChatStreaming'

const props = defineProps<{
  message: Message
  meta: BubbleMeta | null
  isStreaming: boolean
  isDark: boolean
}>()

const store = inject(CHAT_STORE_KEY)!
const { menu, versioning } = store
const { attachmentUrls, billingTotal, billingUnitOf, fmtYuan, billingRowValue, toggleBillingUnit } =
  store.streaming
const { openImagePreview } = store.preview
const { onCopy, onBranch, onSaveTemp, onRollback, onUndoBefore } = versioning

</script>
<template>
<div
  :id="'msg-' + message.id"
  class="msg-bubble"
  :class="[`role-${message.role}`, { streaming: isStreaming }]"
  @pointerdown="menu.onMsgPointerDown($event, message)"
  @pointerup="menu.onMsgPointerUp"
  @pointerleave="menu.onMsgPointerUp"
  @pointercancel="menu.onMsgPointerUp"
  @contextmenu="menu.onMsgContextMenu($event, message)"
>
  <!-- 会话版本操作 hover 操作栏:用量标签 / 复制 / 开启分支 / 保存临时版本 / 回溯版本 / 撤回至此消息前 -->
  <div v-if="!isStreaming" class="msg-hover-bar" @pointerdown.stop>
    <!-- 计费/用量标签:回答结束后显示本次消费,悬浮查看明细 -->
    <div v-if="meta?.billing && billingTotal(meta.billing) > 0" class="msg-usage-wrap">
      <div class="msg-usage" :class="{ priced: meta.billing.priced }">
        <span v-if="billingUnitOf(message.id) === 'price'">
          {{ fmtYuan(meta.billing.total_cost) }}元
        </span>
        <span v-else>{{ billingTotal(meta.billing) }} tokens</span>
      </div>
      <div class="msg-usage-tip">
        <div class="tip-head">
          <span class="tip-title">用量</span>
          <button
            v-if="meta.billing.priced"
            type="button"
            class="tip-toggle"
            @click.stop="toggleBillingUnit(message.id)"
          >
            切换单位为{{ billingUnitOf(message.id) === 'price' ? 'token' : '元' }}
          </button>
        </div>
        <div class="tip-row">
          <span class="tip-label">处理轮数</span>
          <span class="tip-val">{{ meta.billing.rounds }}</span>
        </div>
        <div class="tip-row">
          <span class="tip-label">缓存计费</span>
          <span class="tip-val">{{ billingRowValue(message.id, meta.billing, 'hit') }}</span>
        </div>
        <div class="tip-row">
          <span class="tip-label">未缓存计费</span>
          <span class="tip-val">{{ billingRowValue(message.id, meta.billing, 'miss') }}</span>
        </div>
        <div class="tip-row">
          <span class="tip-label">输出计费</span>
          <span class="tip-val">{{ billingRowValue(message.id, meta.billing, 'output') }}</span>
        </div>
      </div>
    </div>
    <button
      type="button"
      class="msg-hover-btn"
      title="复制信息"
      @click.stop="onCopy(message)"
    >
      <Icon name="copy" :size="14" />
      <span class="msg-hover-tip">复制信息</span>
    </button>
    <button
      type="button"
      class="msg-hover-btn"
      title="开启分支"
      @click.stop="onBranch(message)"
    >
      <Icon name="branch" :size="14" />
      <span class="msg-hover-tip">开启分支：从此消息另起一条对话线</span>
    </button>
    <button
      type="button"
      class="msg-hover-btn"
      title="保存临时版本"
      @click.stop="onSaveTemp(message)"
    >
      <Icon name="bookmark" :size="14" />
      <span class="msg-hover-tip">保存临时版本：在此消息打版本书签</span>
    </button>
    <button
      type="button"
      class="msg-hover-btn"
      title="回溯版本"
      @click.stop="onRollback(message)"
    >
      <Icon name="refresh" :size="14" />
      <span class="msg-hover-tip">回溯版本：对话重置到此消息（其后移除）</span>
    </button>
    <button
      type="button"
      class="msg-hover-btn"
      title="撤回至此消息前"
      @click.stop="onUndoBefore(message)"
    >
      <Icon name="undo" :size="14" />
      <span class="msg-hover-tip">撤回至此消息前：删除此消息及其后全部</span>
    </button>
  </div>
    <template v-if="message.role === 'assistant'">
      <AssistantContent
        :message="message"
        :meta="meta"
        :is-streaming="isStreaming"
        :is-dark="isDark"
      />
      <!-- 附件图片区域:image_gen 工具生成的图片在此渲染 -->
      <div v-if="message.attachments && message.attachments.length > 0" class="msg-attachments">
        <div
          v-for="att in message.attachments"
          :key="att.id"
          class="msg-attachment"
          :class="`att-${att.kind}`"
        >
          <img
            v-if="attachmentUrls[att.id]"
            :src="attachmentUrls[att.id]"
            :alt="att.name"
            class="msg-attachment-img"
            loading="lazy"
            @click="openImagePreview(attachmentUrls[att.id], att.name)"
          />
          <div v-else class="msg-attachment-loading">
            <Icon name="image" :size="20" />
            <span>加载中…</span>
          </div>
          <div class="msg-attachment-meta">{{ att.name }}</div>
        </div>
      </div>
    </template>
    <template v-else>{{ message.content }}</template>
  </div>
</template>

<style scoped>
/* 消息内附件图片区域 */
.msg-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 8px;
}

.msg-attachment {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-width: 260px;
}

.msg-attachment-img {
  max-width: 260px;
  max-height: 200px;
  border-radius: var(--radius-lg);
  object-fit: cover;
  cursor: zoom-in;
  background: var(--bg-2);
  border: 1px solid var(--border);
  transition: filter 0.15s ease, transform 0.15s ease;
}

.msg-attachment-img:hover {
  filter: brightness(1.05);
  transform: scale(1.01);
}

.msg-attachment-loading {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 20px 16px;
  color: var(--muted);
  font-size: 13px;
  background: var(--bg-2);
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
}

.msg-attachment-meta {
  font-size: 12px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 计费/用量标签:嵌入 hover 操作栏左侧(回答结束后显示本次消费),
   悬浮时展开明细浮层 */
.msg-usage-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
  margin-right: 4px;
  cursor: default;
}

.msg-usage {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  background: var(--bg-2);
  border: 1px solid var(--border);
  color: var(--muted);
  white-space: nowrap;
  transition: color 0.15s ease, border-color 0.15s ease;
}

.msg-usage.priced {
  color: var(--success);
}

/* 悬浮明细浮层:操作栏位于消息顶部,明细向下展开避免被遮挡 */
.msg-usage-tip {
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  z-index: 30;
  min-width: 240px;
  padding: 10px 12px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  opacity: 0;
  pointer-events: none;
  transform: translateY(4px);
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.msg-usage-wrap:hover .msg-usage-tip {
  opacity: 1;
  pointer-events: auto;
  transform: translateY(0);
}

.msg-usage-tip .tip-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}

.msg-usage-tip .tip-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.msg-usage-tip .tip-toggle {
  font-size: 12px;
  color: var(--primary);
  background: transparent;
  border: none;
  cursor: pointer;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
}

.msg-usage-tip .tip-toggle:hover {
  background: color-mix(in srgb, var(--primary) 10%, transparent);
}

.msg-usage-tip .tip-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 3px 0;
  font-size: 12px;
}

.msg-usage-tip .tip-label {
  color: var(--muted);
}

/* 消息气泡定位:作为 hover 操作栏的定位基准 */
/* 消息气泡定位:作为 hover 操作栏的定位基准 */
.msg-bubble {
  position: relative;
  /* 新消息入场:轻量 fadeInUp,一次性动画不残留 transform,不影响已渲染列表 */
  animation: msg-bubble-in 300ms var(--ease-out);
}

@keyframes msg-bubble-in {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

@media (prefers-reduced-motion: reduce) {
  .msg-bubble {
    animation: none;
  }
}

/* ---------- 会话版本操作 hover 操作栏 ---------- */
.msg-hover-bar {
  position: absolute;
  top: 6px;
  right: 8px;
  z-index: 20;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 3px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-md);
  opacity: 0;
  pointer-events: none;
  transform: translateY(-3px);
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.msg-bubble:hover .msg-hover-bar {
  opacity: 1;
  pointer-events: auto;
  transform: translateY(0);
}

.msg-hover-btn {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition: color 0.12s ease, background 0.12s ease;
}

.msg-hover-btn:hover {
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 12%, transparent);
}

/* 自定义 tooltip:悬浮按钮时在下方弹出文字提示 */
.msg-hover-tip {
  position: absolute;
  top: calc(100% + 6px);
  left: 50%;
  transform: translateX(-50%);
  z-index: 30;
  width: max-content;
  max-width: 220px;
  padding: 4px 9px;
  font-size: 12px;
  line-height: 1.4;
  color: var(--text);
  text-align: center;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-md);
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.12s ease;
  white-space: normal;
}

.msg-hover-btn:hover .msg-hover-tip {
  opacity: 1;
}

/* 计费明细值 */
.msg-usage-tip .tip-val {
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
</style>
