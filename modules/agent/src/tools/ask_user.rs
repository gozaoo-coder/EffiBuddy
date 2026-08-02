//! ask_user 工具：让 LLM 向用户提问并等待回答
//!
//! 设计要点：
//! - 接收 1-4 个问题，每个问题 2-4 个选项
//! - 通过 `EventBus` 发布 `AskUser` 事件到前端，前端展示选项卡片
//! - 工具调用本身**不同步等待**用户回答：实际回答由前端通过另一个
//!   Tauri 命令回传到会话历史，LLM 在后续轮次中读取
//! - 返回提示信息告知 LLM "已提问，等待回答"
//!
//! # 校验规则
//! - `questions` 数量 1-4
//! - 每个 `question` 文本非空
//! - 每个 `question.options` 数量 2-4
//!
//! # 临界区
//! 仅短暂持有 `conversation_id` 读锁做 clone，锁内无 IO / 重计算。

use std::sync::Arc;

use effisuite_core::{BusEvent, EventBus};
use rig_core::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// 问题数量上限
const MAX_QUESTIONS: usize = 4;
/// 每题选项数量范围
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 4;

/// 单个问题选项
///
/// 字段均为 `String`（24 字节），大小相同，按语义排序。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    /// 选项显示文本（1-5 个词）
    pub label: String,
    /// 选项说明
    pub description: String,
}

/// 单个问题
///
/// 字段按大小降序：三个 24 字节字段在前，1 字节 `Option<bool>` 在后。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    /// 问题正文
    pub question: String,
    /// 简短标签（≤12 字符）
    pub header: String,
    /// 选项列表（2-4 个）
    pub options: Vec<QuestionOption>,
    /// 是否多选，默认 false
    #[serde(default)]
    pub multi_select: Option<bool>,
}

/// 工具参数
#[derive(Deserialize)]
pub struct AskUserArgs {
    pub questions: Vec<Question>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("ask_user error: {0}")]
pub struct AskUserError(String);

/// 向用户提问工具
///
/// 持有：
/// - `event_bus`：前端交互通道（None 时返回友好错误）
/// - `conversation_id`：当前会话 id 句柄（由 Tauri 命令层维护）
pub struct AskUserTool {
    event_bus: Option<Arc<EventBus>>,
    conversation_id: Arc<RwLock<Option<String>>>,
}

impl AskUserTool {
    pub fn new(
        event_bus: Option<Arc<EventBus>>,
        conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            event_bus,
            conversation_id,
        }
    }

    /// 校验问题列表：数量 1-4，每题文本非空，每题选项 2-4 个
    fn validate(questions: &[Question]) -> Result<(), AskUserError> {
        if questions.is_empty() {
            return Err(AskUserError("questions 不能为空".to_string()));
        }
        if questions.len() > MAX_QUESTIONS {
            return Err(AskUserError(format!(
                "questions 数量超出上限：{}（最多 {} 个）",
                questions.len(),
                MAX_QUESTIONS
            )));
        }
        for (i, q) in questions.iter().enumerate() {
            if q.question.trim().is_empty() {
                return Err(AskUserError(format!(
                    "第 {} 个问题的 question 不能为空",
                    i + 1
                )));
            }
            if q.header.trim().is_empty() {
                return Err(AskUserError(format!(
                    "第 {} 个问题的 header 不能为空",
                    i + 1
                )));
            }
            let n = q.options.len();
            if n < MIN_OPTIONS || n > MAX_OPTIONS {
                return Err(AskUserError(format!(
                    "第 {} 个问题的 options 数量 {} 不在范围 [{}-{}] 内",
                    i + 1,
                    n,
                    MIN_OPTIONS,
                    MAX_OPTIONS
                )));
            }
        }
        Ok(())
    }
}

impl Tool for AskUserTool {
    const NAME: &'static str = "ask_user";

    type Error = AskUserError;
    type Args = AskUserArgs;
    type Output = String;

    fn description(&self) -> String {
        "当任务目标不明确、用户意图模糊、或修改方向过多时，主动向用户提出一组选择题以澄清需求。\n\
         \n\
         【何时使用】\n\
         - 用户需求含糊（如\"优化一下\"\"改一下\"\"帮我做\"缺少具体方向）\n\
         - 一次修改涉及多个互斥方向（技术选型 / UI 风格 / 范围边界）\n\
         - 多个方案难以取舍，需要用户偏好确认\n\
         - 任务边界不清，需要确认目标而非盲目执行\n\
         \n\
         【规格】1-4 个问题，每题 2-4 个选项。\n\
         【流程】调用后前端弹出选项卡片，用户选择会作为新消息进入会话，你在后续轮次即可读到回答。\n\
         【约束】不要在同一轮内重复提问；选项要具体可执行，包含明确取舍维度，而非泛泛而谈。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 4,
                    "description": "问题列表（1-4 个）",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "问题正文"
                            },
                            "header": {
                                "type": "string",
                                "maxLength": 12,
                                "description": "简短标签（≤12 字符）"
                            },
                            "options": {
                                "type": "array",
                                "minItems": 2,
                                "maxItems": 4,
                                "description": "选项列表（2-4 个）",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string",
                                            "description": "选项显示文本（1-5 个词）"
                                        },
                                        "description": {
                                            "type": "string",
                                            "description": "选项说明"
                                        }
                                    },
                                    "required": ["label", "description"]
                                }
                            },
                            "multi_select": {
                                "type": "boolean",
                                "default": false,
                                "description": "是否多选，默认 false"
                            }
                        },
                        "required": ["question", "header", "options"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. 校验参数
        Self::validate(&args.questions)?;

        // 2. 检查前端交互通道
        let event_bus = self.event_bus.as_ref().ok_or_else(|| {
            AskUserError("前端交互通道不可用".to_string())
        })?;

        // 3. 读取会话 id（短暂持锁，仅 clone）
        let conversation_id = self
            .conversation_id
            .read()
            .await
            .clone()
            .unwrap_or_default();

        // 4. 序列化问题列表为 JSON Value（前端按 JSON 解析）
        let questions_value = serde_json::to_value(&args.questions)
            .map_err(|e| AskUserError(format!("序列化 questions 失败: {e}")))?;

        // 5. 发布事件（publish 内部仅 broadcast send，无重 IO）
        event_bus.publish(BusEvent::AskUser {
            conversation_id,
            questions: questions_value,
        });

        Ok(format!(
            "已向用户提问（{} 个问题），等待用户回答。",
            args.questions.len()
        ))
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(label: &str, desc: &str) -> QuestionOption {
        QuestionOption {
            label: label.to_string(),
            description: desc.to_string(),
        }
    }

    fn question(text: &str, header: &str, opts: Vec<QuestionOption>) -> Question {
        Question {
            question: text.to_string(),
            header: header.to_string(),
            options: opts,
            multi_select: None,
        }
    }

    fn make_tool(
        bus: Option<Arc<EventBus>>,
        conv: Option<&str>,
    ) -> AskUserTool {
        AskUserTool::new(bus, Arc::new(RwLock::new(conv.map(|s| s.to_string()))))
    }

    #[test]
    fn validate_rejects_empty() {
        let err = AskUserTool::validate(&[]).unwrap_err();
        assert!(err.to_string().contains("不能为空"));
    }

    #[test]
    fn validate_rejects_too_many_questions() {
        let q = question("q", "h", vec![opt("a", "x"), opt("b", "y")]);
        let qs = vec![q; 5];
        let err = AskUserTool::validate(&qs).unwrap_err();
        assert!(err.to_string().contains("超出上限"));
    }

    #[test]
    fn validate_accepts_max_questions() {
        let q = question("q", "h", vec![opt("a", "x"), opt("b", "y")]);
        let qs = vec![q; MAX_QUESTIONS];
        assert!(AskUserTool::validate(&qs).is_ok());
    }

    #[test]
    fn validate_rejects_empty_question_text() {
        let q = question("  ", "h", vec![opt("a", "x"), opt("b", "y")]);
        let err = AskUserTool::validate(&[q]).unwrap_err();
        assert!(err.to_string().contains("question 不能为空"));
    }

    #[test]
    fn validate_rejects_empty_header() {
        let q = question("q", "  ", vec![opt("a", "x"), opt("b", "y")]);
        let err = AskUserTool::validate(&[q]).unwrap_err();
        assert!(err.to_string().contains("header 不能为空"));
    }

    #[test]
    fn validate_rejects_too_few_options() {
        let q = question("q", "h", vec![opt("a", "x")]);
        let err = AskUserTool::validate(&[q]).unwrap_err();
        assert!(err.to_string().contains("不在范围"));
    }

    #[test]
    fn validate_rejects_too_many_options() {
        let q = question("q", "h", vec![
            opt("a", "x"),
            opt("b", "x"),
            opt("c", "x"),
            opt("d", "x"),
            opt("e", "x"),
        ]);
        let err = AskUserTool::validate(&[q]).unwrap_err();
        assert!(err.to_string().contains("不在范围"));
    }

    #[test]
    fn validate_accepts_boundary_options() {
        let q2 = question("q", "h", vec![opt("a", "x"), opt("b", "y")]);
        assert!(AskUserTool::validate(&[q2]).is_ok());

        let q4 = question("q", "h", vec![
            opt("a", "x"),
            opt("b", "x"),
            opt("c", "x"),
            opt("d", "x"),
        ]);
        assert!(AskUserTool::validate(&[q4]).is_ok());
    }

    #[tokio::test]
    async fn call_rejects_without_event_bus() {
        let tool = make_tool(None, Some("conv-1"));
        let args = AskUserArgs {
            questions: vec![question("q", "h", vec![opt("a", "x"), opt("b", "y")])],
        };
        let err = tool.call(args).await.unwrap_err();
        assert!(err.to_string().contains("前端交互通道不可用"));
    }

    #[tokio::test]
    async fn call_publishes_event_and_returns_ok() {
        let bus = Arc::new(EventBus::new(16));
        let mut rx = bus.subscribe();
        let tool = make_tool(Some(Arc::clone(&bus)), Some("conv-42"));

        let args = AskUserArgs {
            questions: vec![question(
                "选择方案",
                "方案",
                vec![opt("A", "快速"), opt("B", "稳妥")],
            )],
        };
        let out = tool.call(args).await.unwrap();
        assert!(out.contains("已向用户提问"));
        assert!(out.contains("1 个问题"));

        let ev = rx.recv().await.unwrap();
        match ev {
            BusEvent::AskUser {
                conversation_id,
                questions,
            } => {
                assert_eq!(conversation_id, "conv-42");
                let arr = questions.as_array().unwrap();
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0]["question"], "选择方案");
                assert_eq!(arr[0]["options"].as_array().unwrap().len(), 2);
            }
            _ => panic!("期望 AskUser 事件，得到 {:?}", ev),
        }
    }

    #[tokio::test]
    async fn call_uses_empty_string_when_no_conversation() {
        let bus = Arc::new(EventBus::new(16));
        let mut rx = bus.subscribe();
        let tool = make_tool(Some(Arc::clone(&bus)), None);

        let args = AskUserArgs {
            questions: vec![question("q", "h", vec![opt("a", "x"), opt("b", "y")])],
        };
        tool.call(args).await.unwrap();

        let ev = rx.recv().await.unwrap();
        match ev {
            BusEvent::AskUser { conversation_id, .. } => {
                assert_eq!(conversation_id, "");
            }
            _ => panic!("期望 AskUser 事件"),
        }
    }

    #[tokio::test]
    async fn call_rejects_invalid_args_before_publishing() {
        let bus = Arc::new(EventBus::new(16));
        let rx = bus.subscribe();
        let tool = make_tool(Some(bus), Some("conv-1"));

        let args = AskUserArgs { questions: vec![] };
        let err = tool.call(args).await.unwrap_err();
        assert!(err.to_string().contains("不能为空"));

        assert!(rx.is_empty());
    }
}
