//! ASR 转写文本摘要生成
//!
//! 调用 [`ChatAgent`](crate::ChatAgent)（非流式）对转写文本生成结构化摘要，
//! 摘要包含一句话总结、关键要点、关键实体、建议标签。
//!
//! # 性能
//!
//! - prompt 模板用 `Cow<'static, str>` 避免无谓分配（模板为静态字符串）
//! - 拼装 prompt 用 `with_capacity` 预分配

use std::borrow::Cow;

use effisuite_core::{Message, Role};

use crate::ChatAgent;

use super::error::AsrError;

/// 摘要 prompt 模板（中文）
const SUMMARY_PROMPT_TEMPLATE: &str = include_str!("summary_prompt.txt");

/// 为转写文本生成结构化摘要
///
/// 调用 `agent.chat`（非流式），传入单条 user 消息（摘要 prompt + 转写文本）。
/// `model` 参数预留（当前使用 agent 当前激活的模型）。
///
/// # 参数
/// - `agent`：任意 ChatAgent 实现（通常是 RigAgent）
/// - `transcript`：转写文本
/// - `model`：预留，指定摘要用模型名（当前忽略，用 agent 当前模型）
pub async fn generate_summary(
    agent: &dyn ChatAgent,
    transcript: &str,
    _model: Option<&str>,
) -> Result<String, AsrError> {
    if transcript.trim().is_empty() {
        return Err(AsrError::Transcribe("转写文本为空，无法生成摘要".into()));
    }

    let prompt = build_summary_prompt(transcript);
    // 构造单条 user 消息：agent.chat 会自动注入 preamble
    let message = Message::new("asr-summary", Role::User, prompt, now_ms());

    let messages = [message];
    agent
        .chat(&messages)
        .await
        .map_err(|e| AsrError::Transcribe(format!("生成摘要失败: {e}")))
}

/// 拼装摘要 prompt：模板 + 转写文本
fn build_summary_prompt(transcript: &str) -> String {
    let template: Cow<'_, str> = Cow::Borrowed(SUMMARY_PROMPT_TEMPLATE);
    let capacity = template.len() + transcript.len() + 16;
    let mut out = String::with_capacity(capacity);
    out.push_str(template.as_ref());
    out.push_str("\n\n转写文本：\n");
    out.push_str(transcript);
    out
}

/// 当前 Unix 毫秒时间戳（失败回退 0）
#[inline]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_contains_transcript() {
        let prompt = build_summary_prompt("这是一段转写文本");
        assert!(prompt.contains("这是一段转写文本"));
        assert!(prompt.contains("转写文本："));
    }

    #[test]
    fn build_prompt_includes_template_sections() {
        let prompt = build_summary_prompt("test");
        // 模板应包含结构化摘要的关键要求
        assert!(prompt.contains("一句话总结"));
        assert!(prompt.contains("关键要点"));
        assert!(prompt.contains("关键实体"));
        assert!(prompt.contains("建议标签"));
    }

    #[test]
    fn build_prompt_empty_transcript() {
        let prompt = build_summary_prompt("");
        // 即使转写为空也能拼装（上层 generate_summary 会拒绝空转写）
        assert!(prompt.contains("转写文本："));
    }
}
