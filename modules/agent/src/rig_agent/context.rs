//! 上下文 prompt 构建
//!
//! 把"获取永久记忆 / RAG 检索 / 技能注入 / 当前对话历史全量格式化"四步拆开，
//! 既给 [`RigAgent::build_contextual_prompt`] 用，也给 [`RigAgent::build_context_preview`] 用，
//! 避免预览面板与实际 prompt 出现实现分叉。
//!
//! 启用 RAG 记忆增强 + 永久记忆时的 prompt 格式：
//! ```text
//! [永久记忆]（用户要求永久记住的内容，请始终遵守/参考）
//! 1. [preference] 用户偏好深色主题
//! 2. 我的工作邮箱是 hr@effisuite.com
//!
//! [相关历史记忆]（来自其他对话，供参考）
//! 1. [会话abc123] [用户] 我们之前聊过 Rust 的异步编程
//! 2. [会话def456] [助手] tokio 使用 work-stealing 调度器...
//!
//! [可用技能]（已安装技能中与当前问题相关...）
//! 1. [weather] Weather — Get current weather forecast
//!
//! [当前对话最近]
//! 用户: 那 tokio 是怎么调度这些 future 的？
//! 助手: tokio 使用 work-stealing 调度器...
//!
//! [当前问题]
//! 用户: 能再详细解释一下 work-stealing 吗？
//! ```
//!
//! - `[永久记忆]` 段：每轮**始终**注入（不依赖检索），来自 PinnedMemoryStore
//! - `[相关历史记忆]` 段：按当前问题做 RAG 检索后注入
//! - `[可用技能]` 段：按当前问题做 RAG 检索后注入（仅 name + description 摘要）
//! - 历史段不再做条数 / 字符截断：保留所有消息完整内容。
//!   长会话的 token 预算由消息压缩系统（compress_message）维护。

use std::borrow::Cow;

use effisuite_core::{Message, Role, apply_compression};

use crate::agent::ContextPreview;

use super::{
    HISTORY_TRUNCATE_CHARS, MEMORY_AUTO_INJECT_LIMIT, RECENT_HISTORY_WITH_MEMORY,
    RigAgent, SKILL_AUTO_INJECT_LIMIT, SKILL_SEARCH_MIN_QUERY_LEN,
};

impl RigAgent {
    /// 把传入的 messages 同步到内部 history（便于工具读取最新上下文）
    pub(super) async fn sync_history(&self, messages: &[Message]) {
        let mut h = self.history.write().await;
        // 简单策略：直接替换为最新快照，避免增量 diff 复杂度
        // 用 with_capacity 减少扩容
        if h.capacity() < messages.len() {
            *h = Vec::with_capacity(messages.len() + 8);
        }
        h.clear();
        h.extend_from_slice(messages);
    }

    /// 构建包含完整对话历史的上下文 prompt
    ///
    /// - `[永久记忆]` 段：每轮**始终**注入（不依赖检索），来自 PinnedMemoryStore
    /// - `[相关历史记忆]` 段：按当前问题做 RAG 检索后注入
    /// - 未启用记忆增强或无相关记忆时退化为旧行为：包含全部当前对话历史
    /// - 长消息会被截断到 800 字符，避免 token 爆炸
    pub(super) async fn build_contextual_prompt(&self, messages: &[Message]) -> String {
        // 复用 build_context_parts 的拆分逻辑，避免与预览面板出现实现分叉
        let parts = match self.build_context_parts(messages).await {
            None => return "hello".to_string(),
            Some(p) => p,
        };

        // 若无永久记忆、无历史且无 RAG 记忆，直接返回当前问题（旧行为）
        if parts.pinned_section.is_empty()
            && parts.history_section.is_empty()
            && parts.memory_section.is_empty()
        {
            return parts.current_question;
        }

        parts.assemble_prompt()
    }

    /// 构建上下文注入预览：返回结构化的各段内容 + 拼装后的完整 prompt
    ///
    /// 与 `build_contextual_prompt` 共享 `build_context_parts` 拆分逻辑，
    /// 确保预览面板展示的内容与实际发给 LLM 的 prompt 完全一致。
    /// 不实际触发 LLM 调用，只读取已注入的永久记忆 / RAG 检索 / 当前对话历史。
    pub async fn build_context_preview(&self, messages: &[Message]) -> ContextPreview {
        let preamble = self.preamble.clone();
        let memory_enabled = self.memory.is_some();
        let skill_auto_inject_enabled = self.skill_index.is_some();

        match self.build_context_parts(messages).await {
            None => ContextPreview {
                preamble,
                pinned_section: String::new(),
                memory_section: String::new(),
                skill_section: String::new(),
                history_section: String::new(),
                current_question: String::new(),
                full_prompt: String::new(),
                pinned_count: 0,
                memory_hits_count: 0,
                skill_hits_count: 0,
                history_keep_count: 0,
                history_total_count: 0,
                memory_inject_limit: MEMORY_AUTO_INJECT_LIMIT,
                recent_history_limit: RECENT_HISTORY_WITH_MEMORY,
                history_truncate_chars: HISTORY_TRUNCATE_CHARS,
                memory_enabled,
                skill_auto_inject_enabled,
            },
            Some(parts) => {
                // 退化为纯当前问题（与 build_contextual_prompt 保持一致）
                let full_prompt = if parts.pinned_section.is_empty()
                    && parts.history_section.is_empty()
                    && parts.memory_section.is_empty()
                    && parts.skill_section.is_empty()
                {
                    parts.current_question.clone()
                } else {
                    parts.assemble_prompt()
                };

                ContextPreview {
                    preamble,
                    pinned_section: parts.pinned_section,
                    memory_section: parts.memory_section,
                    skill_section: parts.skill_section,
                    history_section: parts.history_section,
                    current_question: parts.current_question,
                    full_prompt,
                    pinned_count: parts.pinned_count,
                    memory_hits_count: parts.memory_hits_count,
                    skill_hits_count: parts.skill_hits_count,
                    history_keep_count: parts.history_keep_count,
                    history_total_count: messages.len(),
                    memory_inject_limit: MEMORY_AUTO_INJECT_LIMIT,
                    recent_history_limit: RECENT_HISTORY_WITH_MEMORY,
                    history_truncate_chars: HISTORY_TRUNCATE_CHARS,
                    memory_enabled,
                    skill_auto_inject_enabled,
                }
            }
        }
    }

    /// 拆分 `build_contextual_prompt` 的内部逻辑为可复用结构
    ///
    /// 把"获取永久记忆 / RAG 检索 / 当前对话历史全量格式化"三步拆开，
    /// 既给 `build_contextual_prompt` 用，也给 `build_context_preview` 用，
    /// 避免预览面板与实际 prompt 出现实现分叉。
    ///
    /// 历史段不再做条数 / 字符截断：保留所有消息完整内容。
    /// 长会话的 token 预算由消息压缩系统（compress_message）维护。
    ///
    /// 返回 `None` 表示 `messages` 为空（与原 `build_contextual_prompt` 的早退分支一致）。
    async fn build_context_parts(&self, messages: &[Message]) -> Option<ContextParts> {
        if messages.is_empty() {
            return None;
        }

        // 找到最后一条用户消息的位置
        let last_user_idx = messages
            .iter()
            .rposition(|m| m.role == Role::User)
            .unwrap_or(messages.len() - 1);
        let current_msg = &messages[last_user_idx];

        // 0. 永久记忆段：始终注入（不依赖检索相关性）
        let pinned_section = if let Some(pinned) = &self.pinned_memory {
            pinned.format_for_context().await
        } else {
            String::new()
        };
        // 永久记忆条目数：解析格式化后的字符串行数（含头部说明行），减 1 得条目数
        let pinned_count = if pinned_section.is_empty() {
            0
        } else {
            pinned_section.lines().count().saturating_sub(1)
        };

        // 1. 若启用记忆增强，检索跨会话相关历史
        let (memory_section, memory_hits_count) = if let Some(memory) = &self.memory {
            // 跳过过短查询（如单字符）避免无意义检索
            let query = current_msg.content.trim();
            if query.len() < 2 {
                (String::new(), 0)
            } else {
                let exclude = self.current_conversation_id.read().await.clone();
                let hits = memory
                    .search_hybrid(query, MEMORY_AUTO_INJECT_LIMIT, exclude.as_deref())
                    .await;
                let count = hits.len();
                if hits.is_empty() {
                    (String::new(), 0)
                } else {
                    (format_memory_section(&hits), count)
                }
            }
        } else {
            (String::new(), 0)
        };

        // 1.5. RAG 技能自动注入：检索与当前问题相关的 Top-K 已安装技能
        // 仅注入 name + description 摘要让 agent 知道"我能用什么"，
        // agent 通过 list_installed_skills / get_skill_detail / enable_skill 工具深入使用
        let (skill_section, skill_hits_count) = if let Some(skill_idx) = &self.skill_index {
            let query = current_msg.content.trim();
            if query.len() < SKILL_SEARCH_MIN_QUERY_LEN {
                (String::new(), 0)
            } else {
                let hits = skill_idx.search(query, SKILL_AUTO_INJECT_LIMIT).await;
                let count = hits.len();
                if hits.is_empty() {
                    (String::new(), 0)
                } else {
                    (format_skill_section(&hits), count)
                }
            }
        } else {
            (String::new(), 0)
        };

        // 2. 当前对话历史：全量注入（不截断条数，不截断单条字符）
        // 旧逻辑：启用 RAG 时只取最近 RECENT_HISTORY_WITH_MEMORY 条，单条截断到
        //         HISTORY_TRUNCATE_CHARS 字符。新逻辑：保留所有消息完整内容，
        //         让 LLM 拥有完整的当前对话上下文，避免重要细节被截断丢失。
        //         长会话的 token 预算由消息压缩系统（compress_message）维护，
        //         而非在 build_context_parts 这一层硬截断。
        //
        // 压缩：若注入了 compression_store，加载当前会话的压缩状态并对历史段
        // （messages[..last_user_idx]）应用 Keep/Hide/Replace 决策。
        // 当前问题（最后一条用户消息）不压缩。
        // 用 Cow 避免无压缩状态时的整段克隆（零成本退化）。
        let history_slice: &[Message] = &messages[..last_user_idx];
        let compressed_history: Cow<'_, [Message]> = match &self.compression_store {
            Some(store) => {
                // 读 current_conversation_id 后立即释放锁（临界区极短）
                let conv_id = self.current_conversation_id.read().await.clone();
                match conv_id {
                    Some(id) => match store.load(&id).await {
                        Ok(Some(state)) if !state.actions.is_empty() => {
                            Cow::Owned(apply_compression(history_slice, &state))
                        }
                        Ok(_) => Cow::Borrowed(history_slice),
                        Err(e) => {
                            tracing::warn!(error = %e, "加载压缩状态失败，使用未压缩历史");
                            Cow::Borrowed(history_slice)
                        }
                    },
                    None => Cow::Borrowed(history_slice),
                }
            }
            None => Cow::Borrowed(history_slice),
        };
        let history_msgs: &[Message] = compressed_history.as_ref();

        // 3. 格式化历史段（含 `[当前对话最近]` 头部，全量不截断）
        let history_section = if history_msgs.is_empty() {
            String::new()
        } else {
            // 预估容量：每条平均 128 字节；宁多勿少，避免多次扩容
            let mut s = String::with_capacity(history_msgs.len() * 128 + 32);
            s.push_str("[当前对话最近]\n");
            for m in history_msgs {
                let role_label = match m.role {
                    Role::User => "用户",
                    Role::Assistant => "助手",
                    Role::System => "系统",
                };
                s.push_str(role_label);
                s.push_str(": ");
                s.push_str(&m.content);
                s.push('\n');
            }
            s
        };

        Some(ContextParts {
            pinned_section,
            pinned_count,
            memory_section,
            memory_hits_count,
            skill_section,
            skill_hits_count,
            history_section,
            history_keep_count: history_msgs.len(),
            current_question: current_msg.content.clone(),
        })
    }
}

/// `build_context_parts` 的中间产物：把各段拼装前的内容拆开保存
///
/// 用于 `build_contextual_prompt` 与 `build_context_preview` 共享拆分逻辑。
struct ContextParts {
    pinned_section: String,
    memory_section: String,
    skill_section: String,
    history_section: String,
    current_question: String,
    pinned_count: usize,
    memory_hits_count: usize,
    skill_hits_count: usize,
    history_keep_count: usize,
}

impl ContextParts {
    /// 把各段按
    /// `[永久记忆] → [相关历史记忆] → [可用技能] → [当前对话最近] → [当前问题]`
    /// 顺序拼装。
    ///
    /// `[可用技能]` 段位置选择在历史记忆之后、当前对话之前：
    /// - 不放最前：避免覆盖永久记忆（用户主动要求的高优先级）
    /// - 不放最后：避免与当前问题抢夺注意力，让 agent 先看到"我能用什么"再读问题
    fn assemble_prompt(&self) -> String {
        let mut prompt = String::with_capacity(
            self.pinned_section.len()
                + self.memory_section.len()
                + self.skill_section.len()
                + self.history_section.len()
                + self.current_question.len()
                + 96,
        );

        if !self.pinned_section.is_empty() {
            prompt.push_str(&self.pinned_section);
            prompt.push('\n');
        }
        if !self.memory_section.is_empty() {
            prompt.push_str(&self.memory_section);
            prompt.push('\n');
        }
        if !self.skill_section.is_empty() {
            prompt.push_str(&self.skill_section);
            prompt.push('\n');
        }
        if !self.history_section.is_empty() {
            prompt.push_str(&self.history_section);
            prompt.push('\n');
        }
        prompt.push_str("[当前问题]\n用户: ");
        prompt.push_str(&self.current_question);
        prompt
    }
}

/// 格式化技能自动注入的 `[可用技能]` 段落
///
/// 输出格式：
/// ```text
/// [可用技能]（已安装技能中与当前问题相关，调用 enable_skill(id) 启用）
/// 1. [weather] Weather — Get current weather forecast
/// 2. [translator] Translator — Translate text between languages
/// ```
fn format_skill_section(hits: &[effisuite_core::SkillHit]) -> String {
    let mut s = String::with_capacity(hits.len() * 96 + 64);
    s.push_str("[可用技能]（已安装技能中与当前问题相关，调用 enable_skill(id) 启用；\
                调用 get_skill_detail(id) 查看完整说明；\
                调用 list_installed_skills 查看全部；\
                调用 search_clawhub_skills / install_clawhub_skill 从 ClawHub 找新技能）\n");
    for (i, hit) in hits.iter().enumerate() {
        let tag = if hit.builtin { "[内置]" } else { "" };
        s.push_str(&format!(
            "{}. [{}] {}{} — {}\n",
            i + 1,
            short_skill_id(&hit.id),
            tag,
            hit.name,
            hit.description
        ));
    }
    s
}

/// 截断技能 id 用于显示（取前 12 字符，UTF-8 边界安全）。
/// 比 conversation id 略长，因为技能 id 常是 slug 风格（如 "agent-reach"），
/// 前 12 字符更易辨识
#[inline]
fn short_skill_id(id: &str) -> &str {
    if id.len() <= 12 {
        id
    } else {
        &id[..id.ceil_char_boundary(12)]
    }
}

/// 格式化记忆增强的 `[相关历史记忆]` 段落
///
/// 输出格式：
/// ```text
/// [相关历史记忆]（来自其他对话，供参考）
/// 1. [会话abc12345] [用户] 我们之前聊过 Rust 的异步编程
/// 2. [会话def67890] [助手] tokio 使用 work-stealing 调度器...
/// ```
fn format_memory_section(hits: &[effisuite_core::MemoryHit]) -> String {
    let mut s = String::with_capacity(hits.len() * (effisuite_core::SNIPPET_MAX_CHARS + 48));
    s.push_str("[相关历史记忆]（来自其他对话，供参考）\n");
    for (i, hit) in hits.iter().enumerate() {
        let role = match hit.role {
            Role::User => "用户",
            Role::Assistant => "助手",
            Role::System => "系统",
        };
        s.push_str(&format!(
            "{}. [会话{}] [{}] {}\n",
            i + 1,
            short_conv_id(&hit.conversation_id),
            role,
            hit.snippet
        ));
    }
    s
}

/// 截断会话 id 用于显示（取前 8 字符，UTF-8 边界安全）
#[inline]
fn short_conv_id(id: &str) -> &str {
    if id.len() <= 8 {
        id
    } else {
        &id[..id.ceil_char_boundary(8)]
    }
}
