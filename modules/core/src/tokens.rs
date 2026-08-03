//! 真实 token 指标：一律取自 chat/completions 响应的 `usage` 字段
//!
//! **本项目不做本地分词估算**（不用 OpenAI cl100k_base BPE，也不用
//! 字符数估算）。token 量的大小只能从
//! <https://api.deepseek.com/chat/completions> 的响应里拿——
//! `usage.prompt_tokens` / `usage.completion_tokens` / `usage.total_tokens`
//! 是模型分词器在服务端算出的真实计数，本地无法复现。
//!
//! 这些真实值在每次 completion 后经 [`MessageUsage`] 持久化到对应
//! assistant 消息上（见 tauri 命令层 `persist_assistant_turn`），
//! 本模块只负责从会话历史里把最近一次的真实值取出来，供压缩节省量、
//! 上下文占用等指标使用，保证前端展示的每个 token 数都来自真实响应。

use crate::models::{Message, MessageUsage, Role};

/// 返回会话历史中最后一条「有真实用量」的 assistant 消息 usage。
///
/// 跳过 `input_tokens == 0` 的记录：那是 rig 在 provider 未返回 usage 时的
/// 零值哨兵（`MessageUsage` 仍会持久化，但 input_tokens=0 不代表真实占用），
/// 继续向前找更早的真实上报。
fn last_usage(messages: &[Message]) -> Option<&MessageUsage> {
    messages.iter().rev().find_map(|m| {
        if m.role != Role::Assistant {
            return None;
        }
        match m.usage.as_ref() {
            Some(u) if u.input_tokens > 0 => Some(u),
            _ => None,
        }
    })
}

/// 最近一次 completion 的 API 上报 `prompt_tokens`（即 `MessageUsage.input_tokens`）。
///
/// 该值 = 当时整段 prompt 的真实 token 占用（含系统提示、历史、工具定义、
/// 注入的思维链等），是模型在服务端按自身分词器算出的真实计数，非本地估算。
/// 会话里没有任何带 usage 的消息（如旧版本数据）时返回 `None`。
pub fn last_reported_input_tokens(messages: &[Message]) -> Option<u64> {
    last_usage(messages).map(|u| u.input_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, MessageUsage, Role};

    fn msg(id: &str, role: Role, content: &str) -> Message {
        Message::new(id, role, content, 1)
    }

    fn with_usage(mut m: Message, input_tokens: u64) -> Message {
        m.usage = Some(MessageUsage {
            input_tokens,
            output_tokens: 1,
            total_tokens: input_tokens + 1,
            reasoning_tokens: 0,
            cache_hit_tokens: 0,
            cache_miss_tokens: input_tokens,
            rounds: 1,
        });
        m
    }

    #[test]
    fn empty_history_returns_none() {
        assert_eq!(last_reported_input_tokens(&[]), None);
    }

    #[test]
    fn no_usage_messages_returns_none() {
        let messages = vec![
            msg("m1", Role::User, "hi"),
            msg("m2", Role::Assistant, "hello"),
        ];
        assert_eq!(last_reported_input_tokens(&messages), None);
    }

    #[test]
    fn returns_latest_assistant_usage_input_tokens() {
        let messages = vec![
            msg("m1", Role::User, "hi"),
            with_usage(msg("m2", Role::Assistant, "a"), 100),
            msg("m3", Role::User, "follow up"),
            with_usage(msg("m4", Role::Assistant, "b"), 320),
        ];
        // 取最近一条带真实 usage 的 assistant 消息的 input_tokens
        assert_eq!(last_reported_input_tokens(&messages), Some(320));
    }

    #[test]
    fn skips_zero_input_tokens_usage() {
        // 某些 provider 未上报 usage 时 input_tokens=0，应跳过并继续向前找
        let messages = vec![
            with_usage(msg("m1", Role::Assistant, "a"), 0),
            with_usage(msg("m2", Role::Assistant, "b"), 250),
        ];
        assert_eq!(last_reported_input_tokens(&messages), Some(250));
    }
}
