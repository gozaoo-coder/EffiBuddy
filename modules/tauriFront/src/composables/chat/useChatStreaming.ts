/**
 * 流式渲染状态:气泡元数据 / 流式事件处理 / 附件 / 计费
 *
 * 气泡聚合规则:
 *  - 文本 + 工具调用归同一气泡;工具结果后下一段文本/推理新建气泡
 *  - 连续多个工具无中间文本时,工具调用仍追加到当前气泡(视觉连贯)
 *  - 长程任务回合产生的 assistant 气泡由 TaskMode 聚合隐藏
 */
import { ref, reactive, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { animate } from 'animejs'
import type {
  Message,
  Attachment,
  AgentBillingPayload,
  AgentToolCallPayload,
  AgentToolResultPayload,
  AgentAttachmentPayload,
  SubAgentEventPayload,
  SubAgentRecord,
  ToolCallRecord,
} from '../../types'
import type { useChatCore } from './useChatCore'
import type { useAutoScroll } from './useAutoScroll'

/** 每个助手气泡的元数据:reasoning / tool calls / 子 agent / 计费(流式期间累积,不持久化) */
export interface BubbleMeta {
  reasoning: string
  isThinking: boolean
  toolCalls: ToolCallRecord[]
  /** 子 agent 记录(按 session_id 聚合) */
  subAgents: SubAgentRecord[]
  /** 计费统计:仅在 agent-billing 事件(回答结束时)到达后赋值 */
  billing: AgentBillingPayload | null
}

export function useChatStreaming(
  core: ReturnType<typeof useChatCore>,
  autoscroll: ReturnType<typeof useAutoScroll>,
) {
  const streamingBubbleId = ref<string | null>(null)
  // 工具结果到达后置位:下一个文本/推理 token 应新建气泡,实现"每段答复独立气泡"
  const needNewBubbleAfterTool = ref(false)

  const bubbleMeta = reactive<Record<string, BubbleMeta>>({})

  // 每条消息的计费单位切换状态:'price'(元) / 'token'
  // 未配置价格的模型强制 token 模式,不参与切换。
  const billingUnit = reactive<Record<string, 'price' | 'token'>>({})

  // 附件图片 base64 data URL 缓存:attachment.id -> data URL
  // read_attachment 命令把图片文件编码成 data URL 返回,避免 Tauri 2 资源协议配置。
  const attachmentUrls = reactive<Record<string, string>>({})

  // ---------- meta 访问 ----------
  function getMeta(id: string): BubbleMeta | null {
    return bubbleMeta[id] ?? null
  }

  function ensureMeta(id: string): BubbleMeta {
    if (!bubbleMeta[id]) {
      bubbleMeta[id] = {
        reasoning: '',
        isThinking: false,
        toolCalls: [],
        subAgents: [],
        billing: null,
      }
    }
    return bubbleMeta[id]
  }

  // ---------- 计费单位 ----------
  function billingUnitOf(id: string): 'price' | 'token' {
    const b = bubbleMeta[id]?.billing
    if (b && !b.priced) return 'token'
    return billingUnit[id] ?? 'price'
  }

  function toggleBillingUnit(id: string) {
    billingUnit[id] = billingUnitOf(id) === 'price' ? 'token' : 'price'
  }

  // ---------- 消息渲染 ----------
  // bubble spawn 动画:仅 opacity + scale,不操作 height。
  // 高度变化交给浏览器原生 reflow + markstream-vue 的 smooth-streaming 处理,
  // 避免 ResizeObserver + offsetHeight 在流式场景下的抽搐问题。
  async function addMessage(msg: Message) {
    core.messages.value.push(msg)
    await nextTick()
    const el = document.getElementById('msg-' + msg.id)
    if (!el) {
      autoscroll.scrollBottom()
      return
    }

    // 初始状态:透明 + 缩放 0.96(轻微,避免大幅缩放导致内容模糊)
    el.style.opacity = '0'
    el.style.transform = 'scale(0.96)'
    el.style.transformOrigin = 'center top'
    // 强制 reflow 确保 anime.js 起点准确
    void el.offsetHeight

    animate(el, {
      opacity: [0, 1],
      scale: [0.96, 1],
      duration: 280,
      ease: 'out(3)',
      onComplete: () => {
        el.style.opacity = ''
        el.style.transform = ''
        el.style.transformOrigin = ''
      },
    })

    autoscroll.scrollBottom()
  }

    async function appendStreamToken(token: string) {
      // 工具结果后下一段文本应新建气泡(实现"每段答复独立气泡")
      if (needNewBubbleAfterTool.value) {
        streamingBubbleId.value = null
        needNewBubbleAfterTool.value = false
      }
      // 生成期间用户排队插入的消息:若当前流式气泡之后已有用户气泡,
      // 新一轮回复应另起气泡,避免追加到排队消息之前的旧气泡里。
      if (streamingBubbleId.value) {
        const idx = core.messages.value.findIndex((m) => m.id === streamingBubbleId.value)
        if (core.messages.value.slice(idx + 1).some((m) => m.role === 'user')) {
          streamingBubbleId.value = null
        }
      }
      if (!streamingBubbleId.value) {
        streamingBubbleId.value = core.newId()
        await addMessage({
          id: streamingBubbleId.value,
          role: 'assistant',
          content: token,
          timestamp: Date.now(),
        })
      } else {
        const target = core.messages.value.find((m) => m.id === streamingBubbleId.value)
        if (target) {
          target.content += token
        }
        await nextTick()
        autoscroll.scrollBottom()
      }
      // 收到文本 token 表示推理阶段已结束
      if (streamingBubbleId.value) {
        const meta = bubbleMeta[streamingBubbleId.value]
        if (meta && meta.isThinking) meta.isThinking = false
      }
    }

  // ---------- 推理事件 ----------
    async function onReasoning(content: string) {
      // 工具结果后新一轮推理也应新建气泡(新一轮思考 = 新一段答复)
      if (needNewBubbleAfterTool.value) {
        streamingBubbleId.value = null
        needNewBubbleAfterTool.value = false
      }
      // 生成期间用户排队插入的消息:若当前流式气泡之后已有用户气泡,新一轮思考另起气泡
      if (streamingBubbleId.value) {
        const idx = core.messages.value.findIndex((m) => m.id === streamingBubbleId.value)
        if (core.messages.value.slice(idx + 1).some((m) => m.role === 'user')) {
          streamingBubbleId.value = null
        }
      }
      // 若当前气泡已有正文内容(该段答复已开始/完成),新一轮思考应另起气泡:
      // 避免把新的 thinking 追加混入已有正文气泡,导致 thinking 与正文串位/覆盖。
      if (
        streamingBubbleId.value &&
        core.messages.value.some((m) => m.id === streamingBubbleId.value && m.content)
      ) {
        streamingBubbleId.value = null
      }
      if (!streamingBubbleId.value) {
        // 没有气泡时先创建一个空的 assistant 气泡
        streamingBubbleId.value = core.newId()
        await addMessage({
          id: streamingBubbleId.value,
          role: 'assistant',
          content: '',
          timestamp: Date.now(),
        })
      }
      const meta = ensureMeta(streamingBubbleId.value)
      meta.isThinking = true
      meta.reasoning += content
      await nextTick()
      autoscroll.scrollBottom()
    }

  // ---------- 工具调用事件 ----------
  async function onToolCall(call: AgentToolCallPayload) {
    if (!streamingBubbleId.value) {
      streamingBubbleId.value = core.newId()
      await addMessage({
        id: streamingBubbleId.value,
        role: 'assistant',
        content: '',
        timestamp: Date.now(),
      })
    }
    const meta = ensureMeta(streamingBubbleId.value)
    // 收到 tool call 表示推理阶段结束
    meta.isThinking = false
    meta.toolCalls.push({
      call_id: call.call_id,
      tool_name: call.tool_name,
      arguments: call.arguments,
      result: null,
      is_error: false,
      pending: true,
    })
    await nextTick()
    autoscroll.scrollBottom()
  }

  // ---------- 工具结果事件 ----------
  async function onToolResult(result: AgentToolResultPayload) {
    if (!streamingBubbleId.value) return
    const meta = bubbleMeta[streamingBubbleId.value]
    if (!meta) return
    const target = meta.toolCalls.find((c) => c.call_id === result.call_id)
    if (target) {
      target.result = result.output
      target.is_error = result.is_error
      target.pending = false
    }
    // 标记:下一个文本/推理 token 应新建气泡
    needNewBubbleAfterTool.value = true
    await nextTick()
    autoscroll.scrollBottom()
  }

  // ---------- 附件 ----------
  // 调用 read_attachment 命令把图片文件读成 base64 data URL,缓存到 attachmentUrls。
  async function loadAttachmentDataUrl(att: Attachment) {
    if (attachmentUrls[att.id]) return
    try {
      const dataUrl = await invoke<string>('read_attachment', { path: att.path })
      attachmentUrls[att.id] = dataUrl
    } catch (e) {
      console.warn('read_attachment failed', att.path, e)
    }
  }

  // 批量加载一组消息的所有附件(用于 loadConversation 历史回填)
  async function loadConversationAttachments() {
    const tasks: Promise<void>[] = []
    for (const m of core.messages.value) {
      if (m.attachments && m.attachments.length > 0) {
        for (const att of m.attachments) {
          if (!attachmentUrls[att.id]) tasks.push(loadAttachmentDataUrl(att))
        }
      }
    }
    if (tasks.length > 0) await Promise.all(tasks)
  }

  // ---------- 图片附件事件 ----------
  // image_gen 工具成功生成图片时,后端 emit "agent-attachment" 实时推送 Attachment。
  // 立即挂到当前流式气泡上并加载 base64,用户可在文本生成完成前就看到图片。
  async function onAttachment(payload: AgentAttachmentPayload) {
    if (!streamingBubbleId.value) return
    const target = core.messages.value.find((m) => m.id === streamingBubbleId.value)
    if (!target) return
    if (!target.attachments) target.attachments = []
    // 防止重复推送(同 id 二次到达)
    if (!target.attachments.some((a) => a.id === payload.attachment.id)) {
      target.attachments.push(payload.attachment)
    }
    await loadAttachmentDataUrl(payload.attachment)
    await nextTick()
    autoscroll.scrollBottom()
  }

  // ---------- 子 agent 事件 ----------
  // sub-agent-event:子 agent 全流程事件(started/token/tool_call/tool_result/attachment/done/error)。
  // 按 session_id 聚合到当前流式气泡的子 agent 记录中,前端实时渲染过程卡片。
  async function onSubAgentEvent(p: SubAgentEventPayload) {
    if (!streamingBubbleId.value) {
      streamingBubbleId.value = core.newId()
      await addMessage({
        id: streamingBubbleId.value,
        role: 'assistant',
        content: '',
        timestamp: Date.now(),
      })
    }
    const meta = ensureMeta(streamingBubbleId.value)
    let rec = meta.subAgents.find((s) => s.session_id === p.session_id)
    if (!rec) {
      rec = {
        session_id: p.session_id,
        name: p.name,
        model: p.model,
        depth: p.depth,
        status: 'running',
        task: '',
        text: '',
        toolCalls: [],
        images: [],
        error: '',
        finishedAt: null,
      }
      meta.subAgents.push(rec)
    }
    switch (p.kind) {
      case 'started':
        rec.task = p.content
        rec.status = 'running'
        break
      case 'token':
        rec.text += p.content
        break
      case 'tool_call':
        rec.toolCalls.push({
          call_id: p.session_id + '_' + rec.toolCalls.length,
          tool_name: p.tool_name,
          arguments: p.arguments,
          result: null,
          is_error: false,
          pending: true,
        })
        break
      case 'tool_result': {
        const tc = rec.toolCalls.find((t) => t.tool_name === p.tool_name && t.pending)
        if (tc) {
          tc.result = p.content
          tc.is_error = p.is_error
          tc.pending = false
        }
        break
      }
      case 'attachment':
        try {
          const parsed = JSON.parse(p.content)
          if (parsed.path && parsed.name) {
            rec.images.push({ path: parsed.path, name: parsed.name })
          }
        } catch {
          /* 忽略解析失败 */
        }
        break
      case 'done':
        rec.status = 'done'
        rec.text = p.content || rec.text
        rec.finishedAt = Date.now()
        break
      case 'error':
        rec.status = 'error'
        rec.error = p.content
        rec.finishedAt = Date.now()
        break
    }
    // 同步写入 message.subAgents:右栏概览(用量分析/子代理 token/次数)读 m.subAgents。
    // 赋同一响应式数组引用,后续 push / rec.text += 均能触发面板 computed 重算。
    const target = core.messages.value.find((m) => m.id === streamingBubbleId.value)
    if (target) target.subAgents = meta.subAgents
    await nextTick()
    autoscroll.scrollBottom()
  }

  // ---------- 计费统计(回答结束时) ----------
  // 用户发送一次"询问"后,模型可能因工具调用进行多次 Completions;
  // 全部结束("回答结束")时后端 emit 一次 agent-billing,携带本次询问
  // 的累计用量与按模型配置单价计算的消费金额。
  // 写入"当前 streamingBubbleId"对应的 meta.billing;若介于两段 completion 之间
  // (streamingBubbleId 已被清空),回退到最近一条 assistant 消息。
  async function onBilling(p: AgentBillingPayload) {
    let targetId = streamingBubbleId.value
    if (!targetId) {
      for (let i = core.messages.value.length - 1; i >= 0; i--) {
        if (core.messages.value[i].role === 'assistant') {
          targetId = core.messages.value[i].id
          break
        }
      }
    }
    if (!targetId) return
    const meta = ensureMeta(targetId)
    meta.billing = p
    // 同步写入 message.usage:右栏概览(用量指标/用量分析)读 m.usage 而非 bubbleMeta,
    // 不镜像则流式期间与 agent-billing 到达时均不更新,仅会话重载后才可见。
    const target = core.messages.value.find((m) => m.id === targetId)
    if (target) {
      target.usage = {
        input_tokens: p.cache_hit_tokens + p.cache_miss_tokens,
        output_tokens: p.output_tokens,
        total_tokens: p.total_tokens,
        reasoning_tokens: p.reasoning_tokens,
        cache_hit_tokens: p.cache_hit_tokens,
        cache_miss_tokens: p.cache_miss_tokens,
        rounds: p.rounds,
      }
    }
  }

  // ---------- 计费展示格式化 ----------
  function billingTotal(b: AgentBillingPayload): number {
    return b.cache_hit_tokens + b.cache_miss_tokens + b.output_tokens
  }

  // 元金额格式化:按量级保留有效小数,去掉尾随 0。
  // 例:0.006 -> "0.006";0.00002 -> "0.00002";1.5 -> "1.5"
  function fmtYuan(n: number): string {
    if (!Number.isFinite(n)) return '—'
    if (n === 0) return '0'
    const abs = Math.abs(n)
    let digits: number
    if (abs >= 1) digits = 2
    else if (abs >= 0.01) digits = 4
    else if (abs >= 0.0001) digits = 6
    else digits = 8
    return n.toFixed(digits).replace(/\.?0+$/, '')
  }

  // 计费明细行:单位切换后的显示值
  // - price 模式:"xxx元"(未配置价格时为 "—")
  // - token 模式:"xxx tokens"
  function billingRowValue(
    msgId: string,
    b: AgentBillingPayload,
    kind: 'hit' | 'miss' | 'output',
  ): string {
    const unit = billingUnitOf(msgId)
    if (unit === 'price') {
      const cost =
        kind === 'hit' ? b.cache_hit_cost : kind === 'miss' ? b.cache_miss_cost : b.output_cost
      if (!b.priced) return '—'
      return `${fmtYuan(cost)}元`
    }
    const tokens =
      kind === 'hit' ? b.cache_hit_tokens : kind === 'miss' ? b.cache_miss_tokens : b.output_tokens
    return `${tokens} tokens`
  }

  // ---------- 流式结束 ----------
  async function finalizeStream(full: string) {
    // 分气泡流式:工具结果后已新建多个气泡,full 是全部文本拼接,不能覆盖。
    // 仅在"没有任何气泡"的回退场景下用 full 创建一条消息(防御性兜底)。
    if (!streamingBubbleId.value) {
      if (full) {
        await addMessage({
          id: core.newId(),
          role: 'assistant',
          content: full,
          timestamp: Date.now(),
        })
      }
    }
    // 注:不再用 full 覆盖最后一个气泡 content,分气泡场景下
    // 各气泡已通过 appendStreamToken 正确累积自己的片段。
    streamingBubbleId.value = null
    needNewBubbleAfterTool.value = false
    // 流式结束后通知 App 刷新 SideNav 列表(消息数/时间更新)
    core.emit('conversation-changed')
  }

  // ---------- 历史恢复 ----------
  // 从历史消息恢复气泡元数据(reasoning / toolCalls / usage / subAgents)。
  // 新版已把这些字段持久化到 Message;旧消息(无对应字段)保持默认空状态。
  // 四项互相独立恢复——即使某项缺失也不影响其他项恢复。
  function restoreBubbleMetaFromHistory() {
    for (const m of core.messages.value) {
      if (m.role !== 'assistant') continue
      const meta = ensureMeta(m.id)
      // thinking 全文 → 折叠推理框(历史中视为已思考完成)
      if (m.reasoning) {
        meta.reasoning = m.reasoning
        meta.isThinking = false
      }
      // 工具调用记录 → 工具调用组(历史记录总是已完成,pending=false)
      if (m.toolCalls && m.toolCalls.length > 0) {
        meta.toolCalls = m.toolCalls.map((t) => ({ ...t, pending: false }))
      }
      // token 用量 → 用量显示。历史只持久化 token(价格来自运行时模型配置),
      // 恢复时用当前激活模型的单价重算金额,避免压缩重载后从"元"退化回纯 token 显示。
      if (m.usage) {
        const p = core.activeModelInfo.value?.pricing
        const priced = !!p
        const cache_hit_tokens = m.usage.cache_hit_tokens
        const cache_miss_tokens = m.usage.cache_miss_tokens
        const output_tokens = m.usage.output_tokens
        const cache_hit_cost = priced
          ? (cache_hit_tokens * p!.cache_hit_per_m) / 1_000_000
          : 0
        const cache_miss_cost = priced
          ? (cache_miss_tokens * p!.cache_miss_per_m) / 1_000_000
          : 0
        const output_cost = priced ? (output_tokens * p!.output_per_m) / 1_000_000 : 0
        meta.billing = {
          conversation_id: core.activeId.value ?? '',
          model_name: core.activeModelInfo.value?.name ?? '',
          rounds: m.usage.rounds,
          cache_hit_tokens,
          cache_miss_tokens,
          output_tokens,
          reasoning_tokens: m.usage.reasoning_tokens,
          total_tokens: m.usage.total_tokens,
          priced,
          cache_hit_cost,
          cache_miss_cost,
          output_cost,
          total_cost: cache_hit_cost + cache_miss_cost + output_cost,
        }
      }
      // 子 agent 过程卡片 → 从历史恢复(独立于 usage 恢复)
      if (m.subAgents && m.subAgents.length > 0) {
        meta.subAgents = m.subAgents.map((sa) => ({
          ...sa,
          toolCalls: (sa.toolCalls ?? []).map((t) => ({ ...t, pending: false })),
        }))
      }
    }
  }

  /** 会话切换/清空:清空全部流式与渲染状态 */
  function resetAll() {
    Object.keys(bubbleMeta).forEach((k) => delete bubbleMeta[k])
    Object.keys(billingUnit).forEach((k) => delete billingUnit[k])
    Object.keys(attachmentUrls).forEach((k) => delete attachmentUrls[k])
    streamingBubbleId.value = null
    needNewBubbleAfterTool.value = false
  }

  return {
    streamingBubbleId,
    needNewBubbleAfterTool,
    bubbleMeta,
    billingUnit,
    attachmentUrls,
    getMeta,
    ensureMeta,
    billingUnitOf,
    toggleBillingUnit,
    addMessage,
    appendStreamToken,
    onReasoning,
    onToolCall,
    onToolResult,
    onAttachment,
    onSubAgentEvent,
    onBilling,
    billingTotal,
    fmtYuan,
    billingRowValue,
    finalizeStream,
    restoreBubbleMetaFromHistory,
    loadConversationAttachments,
    resetAll,
  }
}
