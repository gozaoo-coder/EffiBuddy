<script setup lang="ts">
/**
 * AssistantContent —— 助手消息正文渲染(推理 / 工具调用 / 子 agent / Markdown)
 *
 * 被 MessageBubble(普通消息气泡)使用,
 * 只负责「一段助手输出的内容呈现」,不含气泡外壳与交互菜单。
 */
import MarkdownRender from 'markstream-vue'
import ProcessSection from './ProcessSection.vue'
import SubAgentMiniCard from './SubAgentMiniCard.vue'
import type { Message } from '../../types'
import type { BubbleMeta } from '../../composables/chat/useChatStreaming'

defineProps<{
  message: Message
  meta: BubbleMeta | null
  isStreaming: boolean
  isDark: boolean
}>()
</script>

<template>
  <!-- 推理 + 工具调用合并区块：单行摘要标题，进行中展开、完成后自动折叠 -->
  <ProcessSection
    v-if="meta?.segments.length"
    :segments="meta?.segments ?? []"
    :is-thinking="meta?.isThinking ?? false"
  />
    <!-- 子 agent 过程卡片：主视图不作大量片段展开，仅一张紧凑可点卡片，点击进入子代理视图 -->
    <div v-if="meta?.subAgents.length" class="msg-subagents">
      <SubAgentMiniCard
        v-for="sa in meta.subAgents"
        :key="sa.session_id"
        :record="sa"
      />
    </div>
  <!-- 正文:仅在内容非空时渲染(思考/工具阶段内容可能为空) -->
  <MarkdownRender
    v-if="message.content"
    mode="chat"
    :content="message.content"
    :final="!isStreaming"
    :is-dark="isDark"
    :fade="false"
    :smooth-streaming="false"
    :code-block-props="{
      theme: { light: 'vitesse-light', dark: 'vitesse-dark' },
    }"
  />
</template>

<style scoped>
/* 子 agent 过程卡片区:位于过程区块与正文之间 */
.msg-subagents {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 6px;
}
</style>
