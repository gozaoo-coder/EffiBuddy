//! 消息压缩系统：减少 prompt token 占用同时保留关键信息
//!
//! 设计要点（对齐 user_rules 中的 Rust 性能/并发规则）：
//! - 持久化与 [`ConversationStore`] 同构：每会话一 JSON 文件，`RwLock<()>` 仅做并发同步
//! - XML 解析用纯字符串扫描，**不引入 xml crate 依赖**，避免 Cargo.toml 改动
//! - 容错：格式错误的 `<act>` 块以 `tracing::warn!` 记录后跳过，不中断整体解析
//! - `apply_compression` 用 `HashMap<&str, &CompressionAction>` 索引，O(n+m)
//! - 锁临界区极短：`save` 仅在写文件前持写锁，IO 在锁内但无重计算
//! - 结构体字段按大小降序：`Vec`(24B) → `u64`(8B)
//!
//! 压缩对用户透明：UI 仍显示原始消息，仅发给 LLM 的 prompt 中应用压缩决策。
//! `apply_compression` 只在 `RigAgent::build_context_parts` 的历史段调用，
//! 当前问题（最后一条用户消息）不压缩。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{CoreError, Message, Result, Role};

/// `/` 在文件名中的转义序列（与 [`crate::PluginStore`] 一致）
const SLUG_SEP: &str = "__";

/// 单条压缩操作
///
/// 使用 `#[serde(tag = "method", rename_all = "lowercase")]` 让 JSON 表示
/// 携带 `method` 字段作为变体标签，便于前端直接消费：
/// ```json
/// {"method": "keep", "reason": "...", "message_ids": ["id1"]}
/// {"method": "hide", "reason": "...", "message_ids": ["id2"]}
/// {"method": "replace", "reason": "...", "message_ids": ["id3"], "new_content": "..."}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum CompressionAction {
    /// 保持原内容不变
    Keep {
        reason: String,
        message_ids: Vec<String>,
    },
    /// 隐藏（prompt 中不出现，但 UI 仍可见原始消息）
    Hide {
        reason: String,
        message_ids: Vec<String>,
    },
    /// 替换为压缩后的内容
    Replace {
        reason: String,
        message_ids: Vec<String>,
        new_content: String,
    },
}

impl CompressionAction {
    /// 该操作涉及的消息 id 列表（三种变体共享同一字段名）
    #[inline]
    pub fn message_ids(&self) -> &[String] {
        match self {
            CompressionAction::Keep { message_ids, .. }
            | CompressionAction::Hide { message_ids, .. }
            | CompressionAction::Replace { message_ids, .. } => message_ids,
        }
    }

    /// 压缩理由
    #[inline]
    pub fn reason(&self) -> &str {
        match self {
            CompressionAction::Keep { reason, .. }
            | CompressionAction::Hide { reason, .. }
            | CompressionAction::Replace { reason, .. } => reason,
        }
    }
}

/// 一个会话的压缩状态
///
/// 字段按大小降序：`Vec`(24B) → `u64`(8B)。
/// `#[derive(Default)]` 提供 `actions=空 / updated_at=0` 的默认值，
/// 用于"无压缩"场景的零成本退化。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressionState {
    /// 所有 act 操作列表（按时间顺序；后出现的 act 覆盖先出现的）
    pub actions: Vec<CompressionAction>,
    /// 最后一次压缩时间戳（毫秒）
    pub updated_at: u64,
}

/// 压缩状态存储，线程安全可廉价 clone（内部 `RwLock<()>` + `Arc`，与
/// [`crate::ConversationStore`] 同构）
#[derive(Clone)]
pub struct CompressionStore {
    root: PathBuf,
    _lock: Arc<RwLock<()>>,
}

impl CompressionStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: Arc::new(RwLock::new(())),
        })
    }

    /// 将 conversation id 转换为安全文件名：`/` → `__`
    ///
    /// conversation id 通常是 UUID（无 `/`），但保留转义以防未来 id 格式变化，
    /// 并与 [`crate::PluginStore`] 保持一致的防御性策略。
    #[inline]
    fn id_to_file_name(id: &str) -> String {
        id.replace('/', SLUG_SEP)
    }

    /// 压缩状态文件路径：`<root>/<safe_id>.json`
    ///
    /// 二次防御：使用 `Path::file_name` 校验最终路径仍位于 root 内，
    /// 避免 `..` 或路径分隔符导致的目录穿越。
    fn path_for(&self, id: &str) -> PathBuf {
        let safe = Self::id_to_file_name(id);
        let file_name = Path::new(&safe)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(&safe));
        self.root.join(file_name).with_extension("json")
    }

    /// 加载指定会话的压缩状态，不存在返回 None
    pub async fn load(&self, conversation_id: &str) -> Result<Option<CompressionState>> {
        let path = self.path_for(conversation_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let state: CompressionState = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(state))
    }

    /// 保存（或覆盖）指定会话的压缩状态
    pub async fn save(&self, conversation_id: &str, state: &CompressionState) -> Result<()> {
        let _guard = self._lock.write().await;
        let path = self.path_for(conversation_id);
        let bytes = serde_json::to_vec(state).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定会话的压缩状态，不存在返回 Ok(())
    pub async fn delete(&self, conversation_id: &str) -> Result<()> {
        let path = self.path_for(conversation_id);
        if path.exists() {
            tokio::fs::remove_file(&path).await.map_err(CoreError::Io)?;
        }
        Ok(())
    }
}

/// 解析压缩 agent 返回的文本，提取所有 `<act>...</act>` 块
///
/// 容错策略：跳过格式错误的 act 块（记录 `tracing::warn!`），不中断整体解析。
/// 单个 act 内必须包含 `<reason>` / `<method>` / `<completionId>`；
/// 当 `method=替换` 时还须包含 `<newContent>`。
///
/// - `<method>` 严格校验为 `保持` / `隐藏` / `替换`，未知值报错
/// - `<completionId>` 支持 `[id1,id2,...]` 数组语法，容许空白与无括号写法
pub fn parse_compression_response(text: &str) -> Result<Vec<CompressionAction>> {
    let mut actions = Vec::new();
    let mut pos = 0usize;
    while pos < text.len() {
        // 定位下一个 <act>
        let Some(rel_open) = text[pos..].find("<act>") else {
            break;
        };
        let content_start = pos + rel_open + "<act>".len();
        // 定位闭合 </act>
        let Some(rel_close) = text[content_start..].find("</act>") else {
            tracing::warn!("压缩响应存在未闭合的 <act>，剩余内容跳过");
            break;
        };
        let content_end = content_start + rel_close;
        let block = &text[content_start..content_end];
        pos = content_end + "</act>".len();

        match parse_single_act(block) {
            Ok(action) => actions.push(action),
            Err(e) => tracing::warn!(error = %e, "跳过格式错误的 <act> 块"),
        }
    }
    Ok(actions)
}

/// 解析单个 `<act>` 块内容
fn parse_single_act(block: &str) -> Result<CompressionAction> {
    let reason_raw = extract_tag_content(block, "<reason>", "</reason>")
        .ok_or_else(|| CoreError::Agent("<act> 缺少 <reason> 标签".to_string()))?;
    let method_raw = extract_tag_content(block, "<method>", "</method>")
        .ok_or_else(|| CoreError::Agent("<act> 缺少 <method> 标签".to_string()))?;
    let id_raw = extract_tag_content(block, "<completionId>", "</completionId>")
        .ok_or_else(|| CoreError::Agent("<act> 缺少 <completionId> 标签".to_string()))?;

    let message_ids = parse_id_array(id_raw);
    if message_ids.is_empty() {
        return Err(CoreError::Agent(
            "<completionId> 解析后为空数组".to_string(),
        ));
    }

    let reason = reason_raw.trim().to_string();
    let method = method_raw.trim();

    let action = match method {
        "保持" => CompressionAction::Keep {
            reason,
            message_ids,
        },
        "隐藏" => CompressionAction::Hide {
            reason,
            message_ids,
        },
        "替换" => {
            let new_content_raw = extract_tag_content(block, "<newContent>", "</newContent>")
                .ok_or_else(|| {
                    CoreError::Agent("method=替换 缺少 <newContent> 标签".to_string())
                })?;
            CompressionAction::Replace {
                reason,
                message_ids,
                new_content: new_content_raw.trim().to_string(),
            }
        }
        other => {
            return Err(CoreError::Agent(format!(
                "未知 method: {other}（必须是 保持/隐藏/替换）"
            )));
        }
    };
    Ok(action)
}

/// 从 block 中提取 `<tag>...</tag>` 内的内容（不含标签本身）
///
/// 直接传入开/闭标签字符串，避免 `format!` 在调用路径上的堆分配。
/// 找不到任一标签时返回 None。
fn extract_tag_content<'a>(block: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = block.find(open)? + open.len();
    let end = block[start..].find(close)? + start;
    Some(&block[start..end])
}

/// 解析 `<completionId>` 数组语法：`[id1,id2,...]`
///
/// 容错：
/// - 去除首尾 `[` `]` 与空白
/// - 按 `,` 分割后逐项 trim
/// - 过滤空串（如 `[]` 或 `[id1,]`）
fn parse_id_array(raw: &str) -> Vec<String> {
    let stripped = raw.trim().trim_start_matches('[').trim_end_matches(']');
    stripped
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 对消息列表应用压缩决策
///
/// - `Keep` 的消息：保留原样
/// - `Hide` 的消息：从返回列表中移除
/// - `Replace` 的消息：用 `new_content` 替换 `content`，保留 id/role/timestamp/attachments
/// - 未被任何 act 涉及的消息：保留原样
/// - 同一消息被多个 act 命中时：**后出现的 act 覆盖前面的**（按 actions 顺序）
///
/// 性能：先用 `HashMap<&str, &CompressionAction>` 索引（后插入者覆盖先插入者），
/// 再遍历消息查表，总复杂度 O(n+m)。
pub fn apply_compression(messages: &[Message], state: &CompressionState) -> Vec<Message> {
    if state.actions.is_empty() {
        // 无压缩决策：直接克隆原列表（保持调用方语义一致）
        return messages.to_vec();
    }

    // 构建 message_id → &CompressionAction 索引（后插入者覆盖）
    // 容量预估：actions 数 × 平均每 action 涉及 id 数（取 2 倍留余量）
    let mut index: HashMap<&str, &CompressionAction> =
        HashMap::with_capacity(state.actions.len() * 2);
    for action in &state.actions {
        for id in action.message_ids() {
            index.insert(id.as_str(), action);
        }
    }

    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        match index.get(m.id.as_str()) {
            None => out.push(m.clone()),
            Some(CompressionAction::Keep { .. }) => out.push(m.clone()),
            Some(CompressionAction::Hide { .. }) => continue,
            Some(CompressionAction::Replace { new_content, .. }) => {
                let mut new_msg = m.clone();
                new_msg.content = new_content.clone();
                out.push(new_msg);
            }
        }
    }
    out
}

/// 构造发送给压缩 agent 的 prompt（消息列表 + id 标注）
///
/// 包含所有消息（user/assistant/system）及其 id，让压缩 agent 能对任意消息
/// 做决策（用户原始需求明确包含"用户语义表述混乱"的替换场景，故不能只喂 assistant 消息）。
/// 最后一条用户消息标注为"当前问题"，提示压缩 agent 不要对其输出 act。
pub fn build_compression_prompt(messages: &[Message]) -> String {
    // 找到最后一条用户消息位置（与 build_context_parts 一致的"当前问题"定义）
    let last_user_idx = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .unwrap_or(messages.len());

    let mut s = String::with_capacity(messages.len() * 128 + 256);
    s.push_str("以下是当前对话的所有消息，每条都有一个唯一 id。请分析并给出压缩决策。\n\n");
    s.push_str("[消息列表]\n");
    for (i, m) in messages.iter().enumerate() {
        let role = match m.role {
            Role::User => "用户",
            Role::Assistant => "助手",
            Role::System => "系统",
        };
        // write! 直接写入 String，避免 format! 的临时分配
        let _ = std::fmt::Write::write_fmt(
            &mut s,
            format_args!("{}. [id:{}] [{}] ", i + 1, m.id, role),
        );
        if i == last_user_idx {
            s.push_str("(当前问题，无需压缩) ");
        }
        s.push_str(&m.content);
        s.push('\n');
    }
    s.push_str("\n请按照规定的 XML 格式输出压缩决策。不需要压缩的消息可以不输出 act。");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Role;

    // ============ parse_compression_response ============

    #[test]
    fn parse_multiple_acts() {
        let text = r#"
<act>
  <reason>需要保留</reason>
  <method>保持</method>
  <completionId>[m1,m2]</completionId>
</act>
<act>
  <reason>无关话题</reason>
  <method>隐藏</method>
  <completionId>[m3]</completionId>
</act>
<act>
  <reason>语义压缩</reason>
  <method>替换</method>
  <completionId>[m4]</completionId>
  <newContent>简短表述</newContent>
</act>
"#;
        let actions = parse_compression_response(text).unwrap();
        assert_eq!(actions.len(), 3);
        assert!(matches!(
            &actions[0],
            CompressionAction::Keep { message_ids, .. } if message_ids == &["m1".to_string(), "m2".to_string()]
        ));
        assert!(matches!(&actions[1], CompressionAction::Hide { .. }));
        if let CompressionAction::Replace {
            new_content,
            message_ids,
            ..
        } = &actions[2]
        {
            assert_eq!(new_content, "简短表述");
            assert_eq!(message_ids, &["m4".to_string()]);
        } else {
            panic!("expected Replace");
        }
    }

    #[test]
    fn parse_skips_malformed_acts() {
        let text = r#"
<act>
  <reason>正常</reason>
  <method>保持</method>
  <completionId>[m1]</completionId>
</act>
<act>
  <reason>缺少 method</reason>
  <completionId>[m2]</completionId>
</act>
<act>
  <reason>未知 method</reason>
  <method>删除</method>
  <completionId>[m3]</completionId>
</act>
<act>
  <reason>空 completionId</reason>
  <method>隐藏</method>
  <completionId>[]</completionId>
</act>
"#;
        let actions = parse_compression_response(text).unwrap();
        // 仅第一个 act 合法
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], CompressionAction::Keep { .. }));
    }

    #[test]
    fn parse_completion_id_array() {
        let text = r#"
<act>
  <reason>test</reason>
  <method>隐藏</method>
  <completionId>[a, b , c,d ,e]</completionId>
</act>
"#;
        let actions = parse_compression_response(text).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].message_ids(),
            &[
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
                "e".to_string()
            ]
        );
    }

    #[test]
    fn parse_completion_id_without_brackets() {
        // 容错：无方括号也能解析
        let text =
            r#"<act><reason>r</reason><method>保持</method><completionId>x,y</completionId></act>"#;
        let actions = parse_compression_response(text).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].message_ids(),
            &["x".to_string(), "y".to_string()]
        );
    }

    #[test]
    fn parse_replace_with_new_content() {
        let text = r#"<act><reason>r</reason><method>替换</method><completionId>[x]</completionId><newContent>新内容</newContent></act>"#;
        let actions = parse_compression_response(text).unwrap();
        assert_eq!(actions.len(), 1);
        if let CompressionAction::Replace { new_content, .. } = &actions[0] {
            assert_eq!(new_content, "新内容");
        } else {
            panic!("expected Replace");
        }
    }

    #[test]
    fn parse_replace_missing_new_content_errors() {
        let text =
            r#"<act><reason>r</reason><method>替换</method><completionId>[x]</completionId></act>"#;
        let actions = parse_compression_response(text).unwrap();
        assert_eq!(actions.len(), 0); // 缺 newContent 被跳过
    }

    #[test]
    fn parse_empty_text_returns_empty() {
        let actions = parse_compression_response("").unwrap();
        assert!(actions.is_empty());
    }

    #[test]
    fn parse_unclosed_act_skipped() {
        let text =
            r#"<act><reason>r</reason><method>保持</method><completionId>[m1]</completionId>"#;
        // 无 </act> 闭合 → 跳过，返回空
        let actions = parse_compression_response(text).unwrap();
        assert!(actions.is_empty());
    }

    // ============ apply_compression ============

    #[test]
    fn apply_combination_keep_hide_replace() {
        let messages = vec![
            Message::new("m1", Role::User, "hello", 1),
            Message::new("m2", Role::Assistant, "hi", 2),
            Message::new("m3", Role::User, "bye", 3),
            Message::new("m4", Role::Assistant, "goodbye", 4),
        ];
        let state = CompressionState {
            actions: vec![
                CompressionAction::Keep {
                    reason: "r".into(),
                    message_ids: vec!["m1".into()],
                },
                CompressionAction::Hide {
                    reason: "r".into(),
                    message_ids: vec!["m3".into()],
                },
                CompressionAction::Replace {
                    reason: "r".into(),
                    message_ids: vec!["m4".into()],
                    new_content: "短".into(),
                },
            ],
            updated_at: 0,
        };
        let result = apply_compression(&messages, &state);
        // m1 kept, m2 untouched (kept), m3 hidden, m4 replaced
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].id, "m1");
        assert_eq!(result[0].content, "hello");
        assert_eq!(result[1].id, "m2");
        assert_eq!(result[1].content, "hi");
        assert_eq!(result[2].id, "m4");
        assert_eq!(result[2].content, "短");
        // role/timestamp 保留
        assert_eq!(result[2].role, Role::Assistant);
        assert_eq!(result[2].timestamp, 4);
    }

    #[test]
    fn apply_later_action_wins() {
        let messages = vec![Message::new("m1", Role::User, "original", 1)];
        // 同一 id 先 Keep 后 Hide → 后者胜出
        let state = CompressionState {
            actions: vec![
                CompressionAction::Keep {
                    reason: "r".into(),
                    message_ids: vec!["m1".into()],
                },
                CompressionAction::Hide {
                    reason: "r".into(),
                    message_ids: vec!["m1".into()],
                },
            ],
            updated_at: 0,
        };
        let result = apply_compression(&messages, &state);
        assert_eq!(result.len(), 0);

        // 反过来：先 Hide 后 Replace → 后者胜出（m1 保留为替换内容）
        let state2 = CompressionState {
            actions: vec![
                CompressionAction::Hide {
                    reason: "r".into(),
                    message_ids: vec!["m1".into()],
                },
                CompressionAction::Replace {
                    reason: "r".into(),
                    message_ids: vec!["m1".into()],
                    new_content: "替换后".into(),
                },
            ],
            updated_at: 0,
        };
        let result2 = apply_compression(&messages, &state2);
        assert_eq!(result2.len(), 1);
        assert_eq!(result2[0].content, "替换后");
    }

    #[test]
    fn apply_no_actions_keeps_all() {
        let messages = vec![
            Message::new("m1", Role::User, "a", 1),
            Message::new("m2", Role::Assistant, "b", 2),
        ];
        let state = CompressionState::default();
        let result = apply_compression(&messages, &state);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "a");
        assert_eq!(result[1].content, "b");
    }

    #[test]
    fn apply_untouched_messages_kept() {
        let messages = vec![
            Message::new("m1", Role::User, "a", 1),
            Message::new("m2", Role::Assistant, "b", 2),
            Message::new("m3", Role::User, "c", 3),
        ];
        // 只 hide m2，m1/m3 未被涉及
        let state = CompressionState {
            actions: vec![CompressionAction::Hide {
                reason: "r".into(),
                message_ids: vec!["m2".into()],
            }],
            updated_at: 0,
        };
        let result = apply_compression(&messages, &state);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "m1");
        assert_eq!(result[1].id, "m3");
    }

    #[test]
    fn apply_replace_preserves_attachments() {
        let mut m = Message::new("m1", Role::Assistant, "long content", 1);
        m.attachments.push(crate::Attachment {
            id: "a1".into(),
            kind: crate::AttachmentKind::Image,
            path: "img.png".into(),
            name: "img.png".into(),
            mime_type: "image/png".into(),
            size: 100,
        });
        let state = CompressionState {
            actions: vec![CompressionAction::Replace {
                reason: "r".into(),
                message_ids: vec!["m1".into()],
                new_content: "短".into(),
            }],
            updated_at: 0,
        };
        let result = apply_compression(&[m], &state);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "短");
        assert_eq!(result[0].attachments.len(), 1);
        assert_eq!(result[0].attachments[0].id, "a1");
    }

    // ============ build_compression_prompt ============

    #[test]
    fn build_prompt_includes_ids_and_roles() {
        let messages = vec![
            Message::new("id1", Role::User, "你好", 1),
            Message::new("id2", Role::Assistant, "你好啊", 2),
        ];
        let prompt = build_compression_prompt(&messages);
        assert!(prompt.contains("[id:id1]"));
        assert!(prompt.contains("[id:id2]"));
        assert!(prompt.contains("[用户]"));
        assert!(prompt.contains("[助手]"));
        assert!(prompt.contains("你好"));
    }

    #[test]
    fn build_prompt_marks_current_question() {
        let messages = vec![
            Message::new("id1", Role::User, "第一问", 1),
            Message::new("id1b", Role::Assistant, "答1", 2),
            Message::new("id2", Role::User, "第二问", 3),
        ];
        let prompt = build_compression_prompt(&messages);
        // 最后一条用户消息（第二问）应标注"当前问题"
        assert!(prompt.contains("(当前问题，无需压缩)"));
    }

    // ============ CompressionStore 持久化 ============

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "effisuite-compression-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn store_save_load_delete() {
        let dir = tmp_dir();
        let store = CompressionStore::new(&dir).unwrap();

        // 不存在时 load 返回 None
        assert!(store.load("conv1").await.unwrap().is_none());

        // 保存
        let state = CompressionState {
            actions: vec![CompressionAction::Hide {
                reason: "test".into(),
                message_ids: vec!["m1".into(), "m2".into()],
            }],
            updated_at: 12345,
        };
        store.save("conv1", &state).await.unwrap();

        // 加载
        let loaded = store.load("conv1").await.unwrap().unwrap();
        assert_eq!(loaded.actions.len(), 1);
        assert_eq!(loaded.updated_at, 12345);
        assert_eq!(
            loaded.actions[0].message_ids(),
            &["m1".to_string(), "m2".to_string()]
        );

        // 覆盖保存
        let state2 = CompressionState {
            actions: vec![],
            updated_at: 99999,
        };
        store.save("conv1", &state2).await.unwrap();
        let loaded2 = store.load("conv1").await.unwrap().unwrap();
        assert_eq!(loaded2.actions.len(), 0);
        assert_eq!(loaded2.updated_at, 99999);

        // 删除
        store.delete("conv1").await.unwrap();
        assert!(store.load("conv1").await.unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn store_id_filename_safe() {
        let dir = tmp_dir();
        let store = CompressionStore::new(&dir).unwrap();
        // 含 `/` 的 id 应安全转换
        store
            .save(
                "a/b",
                &CompressionState {
                    actions: vec![],
                    updated_at: 0,
                },
            )
            .await
            .unwrap();
        // 文件名应为 a__b.json，而非嵌套路径
        let expected = dir.join("a__b.json");
        assert!(expected.exists());
        // 加载时用原 id
        let loaded = store.load("a/b").await.unwrap().unwrap();
        assert_eq!(loaded.updated_at, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    // ============ serde 往返 ============

    #[test]
    fn action_serde_roundtrip() {
        let actions = vec![
            CompressionAction::Keep {
                reason: "r1".into(),
                message_ids: vec!["a".into(), "b".into()],
            },
            CompressionAction::Hide {
                reason: "r2".into(),
                message_ids: vec!["c".into()],
            },
            CompressionAction::Replace {
                reason: "r3".into(),
                message_ids: vec!["d".into()],
                new_content: "新".into(),
            },
        ];
        for a in &actions {
            let json = serde_json::to_string(a).unwrap();
            let back: CompressionAction = serde_json::from_str(&json).unwrap();
            assert_eq!(a, &back);
        }

        // 验证 tag 字段
        let json = serde_json::to_string(&actions[0]).unwrap();
        assert!(json.contains("\"method\":\"keep\""));
        let json = serde_json::to_string(&actions[1]).unwrap();
        assert!(json.contains("\"method\":\"hide\""));
        let json = serde_json::to_string(&actions[2]).unwrap();
        assert!(json.contains("\"method\":\"replace\""));
        assert!(json.contains("\"new_content\""));
    }

    #[test]
    fn state_serde_roundtrip() {
        let state = CompressionState {
            actions: vec![CompressionAction::Hide {
                reason: "r".into(),
                message_ids: vec!["x".into()],
            }],
            updated_at: 42,
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: CompressionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, back);
    }
}
