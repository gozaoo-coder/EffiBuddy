//! 自动归类 agent：为会话生成简洁标题，并判断它应归入哪个已有文件夹。
//!
//! 独立的 client + 一次性（非流式）调用，复用主对话 agent 的
//! `api_key` / `base_url` / `model_name`，但使用归类专用 preamble。
//!
//! 与 [`crate::rig_agent::compression`] 的设计完全对齐：
//! - 复用 `openai::CompletionsClient`，零状态，天然支持并发
//! - 每次 `build` 一次 agent（builder 零成本，rig 推荐用法）
//! - 返回结构化 `AutoClassifyResult`，由调用方（Tauri 命令层）落盘标题 + 前端更新文件夹映射

use effisuite_core::{CoreError, Message, Result, Role};
use rig_core::{client::CompletionClient, completion::Prompt, providers::openai};
use serde::{Deserialize, Serialize};

/// 标题最大字符数（与 set_title 工具一致，按 Unicode scalar value 计数）
const MAX_TITLE_CHARS: usize = 25;

/// 单条消息在 prompt 中的截断长度（按字符，避免超长消息浪费 token）
const MSG_TRUNCATE_CHARS: usize = 300;

/// 参与归类的消息条数上限（取最近 N 条，足够判断话题即可）
const MAX_MESSAGES: usize = 8;

/// 归类 agent 专用 preamble：说明输出格式（严格 JSON）
pub const AUTO_CLASSIFY_PREAMBLE: &str = "\
你是一个会话分类助手。你的任务是分析聊天记录，为它生成简洁标题，并从已有文件夹中选择最匹配的一个。\n\
\n\
规则：\n\
1. 标题：简洁概括对话主题，不超过 25 个字\n\
2. 文件夹：从用户提供的「已有文件夹」列表中选择最匹配的一个；如果没有合适的文件夹，返回 null\n\
3. 输出格式（严格 JSON，不要包裹 markdown 代码块，不要输出任何额外文字）：\n\
{\"title\":\"标题\",\"folder\":\"文件夹名\"}\n\
或\n\
{\"title\":\"标题\",\"folder\":null}\n\
\n\
注意：\n\
- folder 必须是已有文件夹列表中的某一个（完全匹配名称），或 null\n\
- 如果已有文件夹列表为空，folder 必须为 null\n\
- title 不能为空";

/// 自动归类结果
///
/// - `title`：LLM 生成的标题（已截断到 25 字符）
/// - `folder`：匹配到的已有文件夹名称；无匹配时为 `None`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoClassifyResult {
    pub title: String,
    pub folder: Option<String>,
}

/// 构造归类 prompt：把对话最近 N 条消息 + 已有文件夹列表格式化喂给 LLM
///
/// 设计与 [`effisuite_core::build_compression_prompt`] 一致：
/// - 每条消息标注角色，超长内容按字符截断
/// - 取最近 `MAX_MESSAGES` 条（而非全部），减少 token 消耗
/// - 文件夹列表为空时明确告知 LLM「无已有文件夹」
pub fn build_auto_classify_prompt(messages: &[Message], folders: &[String]) -> String {
    let take_from = messages.len().saturating_sub(MAX_MESSAGES);
    let recent = &messages[take_from..];

    // 预估容量：消息数 * 截断长度 + 文件夹 + 固定头部
    let mut s = String::with_capacity(recent.len() * (MSG_TRUNCATE_CHARS + 32) + folders.len() * 32 + 256);

    s.push_str("请分析以下对话，生成标题并归类。\n\n");

    // 已有文件夹列表
    s.push_str("[已有文件夹]\n");
    if folders.is_empty() {
        s.push_str("（无已有文件夹）\n");
    } else {
        for f in folders {
            s.push_str(f);
            s.push('\n');
        }
    }

    // 对话内容
    s.push_str("\n[对话内容]\n");
    for m in recent {
        let role = match m.role {
            Role::User => "用户",
            Role::Assistant => "助手",
            Role::System => "系统",
        };
        s.push_str(role);
        s.push_str("：");
        let content = m.content.as_str();
        let truncated = truncate_chars(content, MSG_TRUNCATE_CHARS);
        s.push_str(&truncated);
        s.push('\n');
    }

    s.push_str("\n请按照规定的 JSON 格式输出。");
    s
}

/// 调用归类 agent，返回解析后的 `AutoClassifyResult`
///
/// 复用主对话 agent 的 `api_key` / `base_url` / `model_name` + 归类专用 preamble。
/// 非流式调用，适合用户主动触发的单次归类操作。
///
/// # 解析容错
/// LLM 可能在 JSON 外包裹 markdown 代码块或多余文字，这里用 `extract_json` 提取
/// 第一个 `{...}` 块后再反序列化。`folder` 字段会与传入的 `folders` 列表校验，
/// 不匹配时降级为 `None`。
pub async fn call_auto_classify_agent(
    api_key: &str,
    base_url: &str,
    model_name: &str,
    prompt: &str,
    folders: &[String],
) -> Result<AutoClassifyResult> {
    let mut builder = openai::CompletionsClient::builder().api_key(api_key);
    if !base_url.trim().is_empty() {
        builder = builder.base_url(base_url);
    }
    let client = builder
        .build()
        .map_err(|e| CoreError::Agent(format!("auto_classify client init: {e}")))?;

    let agent = client
        .agent(model_name)
        .preamble(AUTO_CLASSIFY_PREAMBLE)
        .build();

    let resp = agent
        .prompt(prompt)
        .await
        .map_err(|e| CoreError::Agent(format!("auto_classify prompt: {e}")))?;

    parse_auto_classify_response(&resp, folders)
}

/// 从 LLM 原始回复中提取并校验归类结果
///
/// 1. 尝试直接解析整段为 JSON
/// 2. 失败则提取第一个 `{...}` 块再解析
/// 3. 标题截断到 25 字符
/// 4. folder 与已有列表校验，不匹配降级为 None
pub fn parse_auto_classify_response(raw: &str, folders: &[String]) -> Result<AutoClassifyResult> {
    let json_str = extract_json(raw);

    #[derive(Deserialize)]
    struct RawResult {
        title: String,
        #[serde(default)]
        folder: Option<serde_json::Value>,
    }

    let parsed: RawResult = serde_json::from_str(json_str)
        .map_err(|e| CoreError::Agent(format!("auto_classify parse JSON: {e}")))?;

    // 标题：trim + 截断
    let title = parsed.title.trim();
    if title.is_empty() {
        return Err(CoreError::Agent("auto_classify: title 不能为空".to_string()));
    }
    let title = truncate_chars(title, MAX_TITLE_CHARS).to_string();

    // 文件夹：serde_json::Value → String，再与已有列表校验
    let folder = parsed.folder.and_then(|v| {
        if v.is_null() {
            return None;
        }
        if let Some(s) = v.as_str() {
            let s = s.trim();
            // 必须完全匹配已有文件夹名
            if folders.iter().any(|f| f == s) {
                return Some(s.to_string());
            }
        }
        None
    });

    Ok(AutoClassifyResult { title, folder })
}

/// 从可能包含 markdown 代码块或多余文字的回复中提取第一个 JSON 对象文本
///
/// 优先匹配 ```` ```json ... ``` ```` 代码块，其次匹配第一个 `{` 到最后一个 `}`。
fn extract_json(raw: &str) -> &str {
    let trimmed = raw.trim();

    // 尝试匹配 ```json ... ``` 或 ``` ... ```
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        // 跳过语言标识（json / text 等）
        let content_start = after_fence
            .find('\n')
            .map(|n| n + 1)
            .unwrap_or(0);
        let content = &after_fence[content_start..];
        if let Some(end) = content.rfind("```") {
            let block = content[..end].trim();
            if !block.is_empty() {
                return block;
            }
        }
    }

    // 回退：取第一个 '{' 到最后一个 '}'
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end >= start {
                return &trimmed[start..=end];
            }
        }
    }

    trimmed
}

/// 按 Unicode scalar value 截断字符串，避免从字节中间切断产生乱码
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_json() {
        let raw = r#"{"title":"Rust 并发","folder":"工作"}"#;
        let folders = vec!["工作".to_string(), "学习".to_string()];
        let r = parse_auto_classify_response(raw, &folders).unwrap();
        assert_eq!(r.title, "Rust 并发");
        assert_eq!(r.folder.as_deref(), Some("工作"));
    }

    #[test]
    fn parse_json_in_markdown_block() {
        let raw = "```json\n{\"title\":\"标题\",\"folder\":null}\n```";
        let r = parse_auto_classify_response(raw, &[]).unwrap();
        assert_eq!(r.title, "标题");
        assert!(r.folder.is_none());
    }

    #[test]
    fn parse_json_with_extra_text() {
        let raw = "好的，以下是结果：\n{\"title\":\"测试\",\"folder\":\"学习\"}\n希望有帮助";
        let folders = vec!["学习".to_string()];
        let r = parse_auto_classify_response(raw, &folders).unwrap();
        assert_eq!(r.title, "测试");
        assert_eq!(r.folder.as_deref(), Some("学习"));
    }

    #[test]
    fn parse_folder_null() {
        let raw = r#"{"title":"新话题","folder":null}"#;
        let folders = vec!["工作".to_string()];
        let r = parse_auto_classify_response(raw, &folders).unwrap();
        assert_eq!(r.title, "新话题");
        assert!(r.folder.is_none());
    }

    #[test]
    fn parse_folder_not_in_list_degrades_to_none() {
        let raw = r#"{"title":"做饭","folder":"生活"}"#;
        let folders = vec!["工作".to_string(), "学习".to_string()];
        let r = parse_auto_classify_response(raw, &folders).unwrap();
        assert_eq!(r.title, "做饭");
        // "生活" 不在已有列表中 → 降级为 None
        assert!(r.folder.is_none());
    }

    #[test]
    fn parse_truncates_overlong_title() {
        let long_title = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十";
        let raw = format!(r#"{{"title":"{}","folder":null}}"#, long_title);
        let r = parse_auto_classify_response(&raw, &[]).unwrap();
        assert_eq!(r.title.chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn parse_rejects_empty_title() {
        let raw = r#"{"title":"   ","folder":null}"#;
        let res = parse_auto_classify_response(raw, &[]);
        assert!(res.is_err());
    }

    #[test]
    fn parse_rejects_invalid_json() {
        let raw = "这不是 JSON";
        let res = parse_auto_classify_response(raw, &[]);
        assert!(res.is_err());
    }

    #[test]
    fn build_prompt_includes_folders() {
        let messages = vec![
            Message {
                id: "m1".to_string(),
                content: "如何写 Rust 异步代码".to_string(),
                timestamp: 0,
                role: Role::User,
                attachments: vec![],
                reasoning: None,
                tool_calls: vec![],
                usage: None,
                sub_agents: vec![],
            },
            Message {
                id: "m2".to_string(),
                content: "使用 tokio 和 async/await".to_string(),
                timestamp: 1,
                role: Role::Assistant,
                attachments: vec![],
                reasoning: None,
                tool_calls: vec![],
                usage: None,
                sub_agents: vec![],
            },
        ];
        let folders = vec!["编程".to_string()];
        let prompt = build_auto_classify_prompt(&messages, &folders);

        assert!(prompt.contains("[已有文件夹]"));
        assert!(prompt.contains("编程"));
        assert!(prompt.contains("[对话内容]"));
        assert!(prompt.contains("用户：如何写 Rust 异步代码"));
        assert!(prompt.contains("助手：使用 tokio 和 async/await"));
    }

    #[test]
    fn build_prompt_empty_folders() {
        let messages = vec![Message {
            id: "m1".to_string(),
            content: "test".to_string(),
            timestamp: 0,
            role: Role::User,
            attachments: vec![],
            reasoning: None,
            tool_calls: vec![],
            usage: None,
            sub_agents: vec![],
        }];
        let prompt = build_auto_classify_prompt(&messages, &[]);
        assert!(prompt.contains("（无已有文件夹）"));
    }

    #[test]
    fn build_prompt_truncates_long_message() {
        let long_content = "a".repeat(MSG_TRUNCATE_CHARS + 100);
        let messages = vec![Message {
            id: "m1".to_string(),
            content: long_content,
            timestamp: 0,
            role: Role::User,
            attachments: vec![],
            reasoning: None,
            tool_calls: vec![],
            usage: None,
            sub_agents: vec![],
        }];
        let prompt = build_auto_classify_prompt(&messages, &[]);
        // 截断后不应包含完整内容
        assert!(!prompt.contains(&"a".repeat(MSG_TRUNCATE_CHARS + 100)));
    }

    #[test]
    fn build_prompt_takes_recent_messages() {
        let messages: Vec<Message> = (0..20)
            .map(|i| Message {
                id: format!("m{}", i),
                content: format!("msg{}", i),
                timestamp: i,
                role: Role::User,
                attachments: vec![],
                reasoning: None,
                tool_calls: vec![],
                usage: None,
                sub_agents: vec![],
            })
            .collect();

        let prompt = build_auto_classify_prompt(&messages, &[]);
        // 只取最近 MAX_MESSAGES 条
        assert!(prompt.contains("msg12"));
        assert!(prompt.contains("msg19"));
        assert!(!prompt.contains("msg11"));
    }

    #[test]
    fn extract_json_handles_plain_object() {
        assert_eq!(extract_json(r#"{"a":1}"#), r#"{"a":1}"#);
    }

    #[test]
    fn extract_json_handles_code_block() {
        let raw = "```json\n{\"a\":1}\n```";
        assert_eq!(extract_json(raw), r#"{"a":1}"#);
    }

    #[test]
    fn extract_json_handles_surrounding_text() {
        let raw = "结果：\n{\"a\":1}\n结束";
        assert_eq!(extract_json(raw), r#"{"a":1}"#);
    }
}
