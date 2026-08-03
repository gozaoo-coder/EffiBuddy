//! 流式聊天事件 payload 定义、token 用量统计、计费汇总与图片输出解析。
//!
//! 这些类型仅供 `chat` 模块内部的流式命令使用，通过 `pub(super)` 暴露给父模块。
//! 字段按大小降序排列（u64/f64 → u32 → bool），最小化结构体 padding。

use effisuite_core::{Attachment, AttachmentKind, ModelPricing};

/// 流式 token payload（与前端 TS 类型对齐）
#[derive(Debug, serde::Serialize)]
pub(super) struct StreamTokenPayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) content: &'a str,
    pub(super) done: bool,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct StreamErrorPayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) error: &'a str,
}

/// agent 被用户手动停止（agent-stopped 事件）
///
/// 携带已产生（已持久化）的部分回复内容，前端据此落盘流式气泡并复位发送状态。
#[derive(Debug, serde::Serialize)]
pub(super) struct AgentStoppedPayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) content: &'a str,
}
/// 推理增量 payload（agent-reasoning 事件）
#[derive(Debug, serde::Serialize)]
pub(super) struct AgentReasoningPayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) content: &'a str,
}

/// 工具调用开始 payload（agent-tool-call 事件）
#[derive(Debug, serde::Serialize)]
pub(super) struct AgentToolCallPayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) call_id: &'a str,
    pub(super) tool_name: &'a str,
    /// JSON 字符串形式的参数
    pub(super) arguments: &'a str,
}

/// 本轮对话累计 token 使用统计（agent-usage 事件携带单次 + 累计值）
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct UsageSummary {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) reasoning_tokens: u64,
    /// 缓存命中输入 token 累计（DeepSeek prompt_cache_hit_tokens）
    pub(super) cache_hit_tokens: u64,
    /// 缓存未命中输入 token 累计（DeepSeek prompt_cache_miss_tokens）
    pub(super) cache_miss_tokens: u64,
    /// 处理轮数：本轮所有 completion 次数（含工具调用轮）
    pub(super) completion_count: u32,
}

/// 回答结束时的计费统计（agent-billing 事件 payload）
///
/// 本轮"询问"（可能包含多次 completion + 工具调用）结束时 emit 一次，
/// 前端据此在气泡底部显示最终消费价格，悬浮可查看分项明细。
#[derive(Debug, serde::Serialize)]
pub(super) struct AgentBillingPayload<'a> {
    pub(super) conversation_id: &'a str,
    /// 模型名（agent 实际使用的模型）
    pub(super) model_name: &'a str,
    /// 处理轮数：本轮所有 completion 次数
    pub(super) rounds: u32,
    /// 缓存命中输入 token 总数
    pub(super) cache_hit_tokens: u64,
    /// 缓存未命中输入 token 总数
    pub(super) cache_miss_tokens: u64,
    /// 输出 token 总数
    pub(super) output_tokens: u64,
    /// 思维链（reasoning）token 总数
    pub(super) reasoning_tokens: u64,
    /// 总 token 数（缓存命中 + 未命中 + 输出）
    pub(super) total_tokens: u64,
    /// 是否已配置计费单价；false 时各 cost 字段为 0，前端只显示 token
    pub(super) priced: bool,
    /// 缓存计费（元）
    pub(super) cache_hit_cost: f64,
    /// 未缓存计费（元）
    pub(super) cache_miss_cost: f64,
    /// 输出计费（元）
    pub(super) output_cost: f64,
    /// 合计消费（元）
    pub(super) total_cost: f64,
}

/// 一次"回答结束"的计费计算结果
///
/// 字段按大小降序：u64/f64(8B) → u32(4B) → bool(1B)，最小化 padding。
#[derive(Debug, Clone, Copy)]
pub(super) struct BillingSummary {
    pub(super) cache_hit_tokens: u64,
    pub(super) cache_miss_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) reasoning_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) cache_hit_cost: f64,
    pub(super) cache_miss_cost: f64,
    pub(super) output_cost: f64,
    pub(super) total_cost: f64,
    pub(super) rounds: u32,
    pub(super) priced: bool,
}

impl BillingSummary {
    /// 根据累计用量与用户配置的计费单价（元/百万 tokens）计算消费金额。
    ///
    /// - 缓存未命中数优先用 provider 上报值；未上报时（如 OpenAI 风格
    ///   provider 只报 cached_tokens）用 `输入 - 缓存命中` 推导。
    /// - `pricing` 为 None（模型未配置单价）时 `priced=false`，不计算金额。
    pub(super) fn compute(summary: &UsageSummary, pricing: Option<ModelPricing>) -> Self {
        let cache_hit_tokens = summary.cache_hit_tokens;
        let cache_miss_tokens = if summary.cache_miss_tokens > 0 {
            summary.cache_miss_tokens
        } else {
            summary.input_tokens.saturating_sub(cache_hit_tokens)
        };
        let output_tokens = summary.output_tokens;
        let total_tokens = cache_hit_tokens + cache_miss_tokens + output_tokens;

        let (priced, cache_hit_cost, cache_miss_cost, output_cost) = match pricing {
            Some(p) => (
                true,
                cache_hit_tokens as f64 * p.cache_hit_per_m / 1_000_000.0,
                cache_miss_tokens as f64 * p.cache_miss_per_m / 1_000_000.0,
                output_tokens as f64 * p.output_per_m / 1_000_000.0,
            ),
            None => (false, 0.0, 0.0, 0.0),
        };

        Self {
            cache_hit_tokens,
            cache_miss_tokens,
            output_tokens,
            reasoning_tokens: summary.reasoning_tokens,
            total_tokens,
            cache_hit_cost,
            cache_miss_cost,
            output_cost,
            total_cost: cache_hit_cost + cache_miss_cost + output_cost,
            rounds: summary.completion_count,
            priced,
        }
    }
}

/// token 使用统计 payload（agent-usage 事件）
///
/// 同时下发"本次单次"与"本轮累计"两组数据：
/// - 单次值：当前 completion 的 token 消耗
/// - 累计值：本轮所有 completion 的总和，前端可直接显示
#[derive(Debug, serde::Serialize)]
pub(super) struct AgentUsagePayload<'a> {
    pub(super) conversation_id: &'a str,
    // 本次单次值
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) reasoning_tokens: u64,
    // 本轮累计值
    pub(super) cumulative_input: u64,
    pub(super) cumulative_output: u64,
    pub(super) cumulative_total: u64,
    pub(super) cumulative_reasoning: u64,
}

/// 工具执行结果 payload（agent-tool-result 事件）
#[derive(Debug, serde::Serialize)]
pub(super) struct AgentToolResultPayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) call_id: &'a str,
    pub(super) output: &'a str,
    pub(super) is_error: bool,
}

/// 图片附件生成 payload（agent-attachment 事件）
///
/// 当 image_gen 工具成功生成图片时实时 emit，前端收到后立即渲染图片，
/// 无需等待流结束。
#[derive(Debug, serde::Serialize)]
pub(super) struct AgentAttachmentPayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) attachment: &'a Attachment,
}

/// 会话标题更新 payload（conversation-title-updated 事件）
///
/// 当 set_title 工具成功更新标题后实时 emit，前端立即刷新 SideNav 列表，
/// 无需等流结束。title 为 SetTitleTool 返回的截断后标题。
#[derive(Debug, serde::Serialize)]
pub(super) struct ConversationTitlePayload<'a> {
    pub(super) conversation_id: &'a str,
    pub(super) title: &'a str,
}

/// 解析 image_gen 工具输出为 Attachment。
///
/// ImageGenTool 返回的 ImageGenOutput 序列化为 JSON：
/// `{"id":"...","path":"gen_xxx.png","name":"生成图片_xxx.png","elapsed_ms":1234}`
/// rig 把它作为 ToolResultContent::Text 传回，extract_tool_output 提取为字符串。
/// 此函数尝试反序列化并构造 Attachment；失败时返回 None（静默跳过）。
pub(super) fn parse_image_gen_output(output: &str) -> Option<Attachment> {
    let v: serde_json::Value = serde_json::from_str(output).ok()?;
    let id = v.get("id")?.as_str()?.to_string();
    let path = v.get("path")?.as_str()?.to_string();
    let name = v.get("name")?.as_str()?.to_string();
    Some(Attachment {
        id,
        kind: AttachmentKind::Image,
        path,
        name,
        mime_type: "image/png".to_string(),
        size: 0,
    })
}
