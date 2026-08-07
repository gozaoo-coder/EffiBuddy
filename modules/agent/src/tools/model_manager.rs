//! manage_model 工具：让 agent 自主管理"可使用模型"列表
//!
//! 能力：
//! - **list**：查看模型列表（含当前激活标记）
//! - **save**：新增 / 更新一个模型（id 为空自动生成）
//! - **delete**：删除模型（若为激活模型会同时清空激活标记）
//! - **activate**：激活模型为当前对话模型（kind=chat）或图像生成模型（kind=image_gen）
//!
//! 配置变更通过共享的 `ModelManagerHandle` 写回 AppState.config 并持久化，
//! 同时 bump 配置版本号，触发 Tauri 层在下一轮 send_message 时懒重建 agent
//! （对话模型激活/编辑在下一轮对话生效，图像模型在下一轮起对 image_gen 工具生效）。
//!
//! 注：本工具只改模型列表与激活标记，不修改运行时内联配置字段，
//! 避免 agent 把自己当前的 api_key/base_url 改坏。

use std::sync::Arc;

use effisuite_core::{AgentConfig, AvailableModel, ModelKind};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 配置持久化回调类型
pub type SaveConfigFn = Box<dyn Fn(&AgentConfig) -> std::result::Result<(), String> + Send + Sync>;

/// 模型管理句柄：由 Tauri 层构造并注入，工具经它安全地修改共享配置
pub struct ModelManagerHandle {
    /// 共享的 AgentConfig 快照（与 AppState.config 同一份）
    /// `Arc<RwLock<Arc<AgentConfig>>>`：读 clone Arc（廉价），写 COW（clone 内部 → 改 → 写回新 Arc）
    pub config: Arc<RwLock<Arc<AgentConfig>>>,
    /// 持久化回调（Tauri 层：写入 config.json）
    pub save: SaveConfigFn,
    /// 配置版本号 bump：Tauri 层据此懒重建 agent（与 AppState.config_rev 同一份）
    pub bump: Arc<std::sync::atomic::AtomicU64>,
}

/// 操作类型
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAction {
    /// 列出全部模型
    List,
    /// 新增 / 更新模型（model 必填）
    Save,
    /// 删除模型（id 必填）
    Delete,
    /// 激活模型为对话或图像生成模型（id 必填）
    Activate,
}

/// 工具参数
#[derive(Deserialize)]
pub struct ManageModelArgs {
    /// 操作类型：list / save / delete / activate
    pub action: ModelAction,
    /// save 操作必填：要新增或更新的模型
    #[serde(default)]
    pub model: Option<AvailableModel>,
    /// delete / activate 操作必填：模型 id
    #[serde(default)]
    pub id: Option<String>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("manage_model error: {0}")]
pub struct ManageModelError(String);

/// 模型管理工具
pub struct ManageModelTool {
    handle: Arc<ModelManagerHandle>,
}

impl ManageModelTool {
    pub fn new(handle: Arc<ModelManagerHandle>) -> Self {
        Self { handle }
    }
}

/// 获取模型的可读能力标签
fn kind_label(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Chat => "chat",
        ModelKind::ImageGen => "image_gen",
        ModelKind::VideoGen => "video_gen",
        ModelKind::AudioTranscribe => "audio_transcribe",
    }
}

impl Tool for ManageModelTool {
    const NAME: &'static str = "manage_model";

    type Error = ManageModelError;
    type Args = ManageModelArgs;
    type Output = String;

  fn description(&self) -> String {
      "管理'可使用模型'列表（模型配置面板同一份数据）：\
       list 查看全部；save 新增/更新（label/base_url/model_name/api_key/kind 等）；delete 删除；\
       activate 激活（chat 激活为对话模型，image_gen 激活为图像模型）。\
       用户要求'换个模型/添加模型/删除模型'时使用；激活对话模型下一轮生效，变更持久化到配置文件。"
          .to_string()
  }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "save", "delete", "activate"],
                      "description": "操作类型：list/save/delete/activate"
                },
                "model": {
                    "type": "object",
                      "description": "save 必填。字段：label, provider_id, base_url, model_name, api_key, kind(chat|image_gen)，其余可选"
                },
                "id": {
                    "type": "string",
                    "description": "delete/activate 必填：模型 id"
                }
            },
            "required": ["action"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match args.action {
            ModelAction::List => self.list().await,
            ModelAction::Save => {
                let model = args
                    .model
                    .ok_or_else(|| ManageModelError("save 操作必须提供 model 参数".into()))?;
                self.save(model).await
            }
            ModelAction::Delete => {
                let id = args
                    .id
                    .ok_or_else(|| ManageModelError("delete 操作必须提供 id 参数".into()))?;
                self.delete(&id).await
            }
            ModelAction::Activate => {
                let id = args
                    .id
                    .ok_or_else(|| ManageModelError("activate 操作必须提供 id 参数".into()))?;
                self.activate(&id).await
            }
        }
    }
}

impl ManageModelTool {
    /// 列出全部模型：格式化为可读文本，标注当前激活项
    async fn list(&self) -> Result<String, ManageModelError> {
        let config = self.handle.config.read().await;
        if config.models.is_empty() {
            return Ok("模型列表为空。可调用 manage_model(save) 添加，或在设置面板中配置。".to_string());
        }
        let mut out = String::with_capacity(config.models.len() * 96 + 64);
        out.push_str(&format!("可使用模型（共 {} 个）：\n", config.models.len()));
        for m in &config.models {
            let active_chat = config.active_model_id.as_deref() == Some(m.id.as_str());
            let active_img = config.active_image_gen_model_id.as_deref() == Some(m.id.as_str());
            let active = if active_chat {
                " [当前对话模型]"
            } else if active_img {
                " [当前图像模型]"
            } else {
                ""
            };
            let tools = if m.enable_tools { "tools" } else { "no-tools" };
            out.push_str(&format!(
                "- id={} kind={} model={} label={} provider={} {}{}\n",
                m.id,
                kind_label(m.kind),
                m.model_name,
                if m.label.is_empty() { "(未命名)" } else { &m.label },
                m.provider_id,
                tools,
                active,
            ));
        }
        out.push_str("\n提示：activate 可切换当前对话模型/图像模型；save 可新增或更新模型。");
        Ok(out)
    }

    /// 新增或更新模型（id 为空自动生成），bump 配置版本
    async fn save(&self, model: AvailableModel) -> Result<String, ManageModelError> {
        // COW：读快照 → clone 内部 → 修改 → 持久化 → 写回新 Arc
        let mut config = self.handle.config.read().await.as_ref().clone();
        let id = if model.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            model.id.clone()
        };
        let kind = model.kind;
        let model_name = model.model_name.clone();
        let label = model.label.clone();
        let mut m = model;
        m.id = id.clone();

        let is_update = config.models.iter().any(|x| x.id == id);
        if let Some(existing) = config.models.iter_mut().find(|x| x.id == id) {
            *existing = m;
        } else {
            config.models.push(m);
        }
        self.persist(&config)?;
        *self.handle.config.write().await = Arc::new(config);
        Ok(format!(
            "已{}模型：id={id} kind={} model={} label={}（已持久化）",
            if is_update { "更新" } else { "新增" },
            kind_label(kind),
            model_name,
            if label.is_empty() { "(未命名)" } else { &label },
        ))
    }

    /// 删除模型；若为激活模型则清空对应激活标记
    async fn delete(&self, id: &str) -> Result<String, ManageModelError> {
        // COW：读快照 → clone → 修改 → 持久化 → 写回新 Arc
        let mut config = self.handle.config.read().await.as_ref().clone();
        let existed = config.models.iter().any(|m| m.id == id);
        if !existed {
            return Err(ManageModelError(format!("模型 {id} 不存在，无需删除")));
        }
        config.models.retain(|m| m.id != id);
        if config.active_model_id.as_deref() == Some(id) {
            config.active_model_id = None;
        }
        if config.active_image_gen_model_id.as_deref() == Some(id) {
            config.active_image_gen_model_id = None;
        }
        self.persist(&config)?;
        *self.handle.config.write().await = Arc::new(config);
        Ok(format!("已删除模型 {id}（若它曾是激活模型，激活标记已清空）"))
    }

    /// 激活模型：chat → 当前对话模型；image_gen → 当前图像生成模型
    async fn activate(&self, id: &str) -> Result<String, ManageModelError> {
        // COW：读快照 → clone → 修改 → 持久化 → 写回新 Arc
        let mut config = self.handle.config.read().await.as_ref().clone();
        let model = config
            .models
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| ManageModelError(format!("模型 {id} 不存在，请先用 list 查看可用 id")))?;
        match model.kind {
            ModelKind::Chat => {
                config.active_model_id = Some(id.to_string());
                self.persist(&config)?;
                *self.handle.config.write().await = Arc::new(config);
                Ok(format!(
                    "已激活对话模型：{}（{id}）。下一轮对话起生效。",
                    model.model_name
                ))
            }
            ModelKind::ImageGen => {
                config.active_image_gen_model_id = Some(id.to_string());
                self.persist(&config)?;
                *self.handle.config.write().await = Arc::new(config);
                Ok(format!(
                    "已激活图像生成模型：{}（{id}）。下一轮起 image_gen 工具将使用此模型。",
                    model.model_name
                ))
            }
            ModelKind::VideoGen => {
                Err(ManageModelError("视频生成模型暂未实现".into()))
            }
            ModelKind::AudioTranscribe => {
                Err(ManageModelError(
                    "音频转文字模型暂不支持通过 manage_model 激活，请在设置中配置".into(),
                ))
            }
        }
    }

    /// 持久化 + bump 版本号
    fn persist(&self, config: &AgentConfig) -> Result<(), ManageModelError> {
        (self.handle.save)(config)
            .map_err(|e| ManageModelError(format!("持久化配置失败: {e}")))?;
        self.handle.bump.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handle() -> (Arc<ModelManagerHandle>, Arc<RwLock<Arc<AgentConfig>>>) {
        let config = Arc::new(RwLock::new(Arc::new(AgentConfig::default())));
        let bump = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let handle = Arc::new(ModelManagerHandle {
            config: Arc::clone(&config),
            save: Box::new(|_| Ok(())),
            bump,
        });
        (handle, config)
    }

    fn sample_model(id: &str) -> AvailableModel {
        AvailableModel {
            id: id.to_string(),
            label: "测试模型".to_string(),
            provider_id: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            model_name: "gpt-4o-mini".to_string(),
            api_key: "sk-test".to_string(),
            preamble: String::new(),
            enable_tools: true,
            kind: ModelKind::Chat,
            image_size: None,
            image_quality: None,
            video_resolution: None,
            video_ratio: None,
            audio_language: None,
            context_window_tokens: Some(128000),
            video_duration: None,
            pricing: None,
            created_at: 1,
        }
    }

    #[tokio::test]
    async fn save_then_list_then_activate_then_delete() {
        let (handle, config) = make_handle();
        let tool = ManageModelTool::new(handle);

        // save（空 id → 自动生成）
        let mut m = sample_model("");
        let out = tool
            .call(ManageModelArgs {
                action: ModelAction::Save,
                model: Some(m.clone()),
                id: None,
            })
            .await
            .unwrap();
        assert!(out.contains("已新增模型"));
        let saved_id = config.read().await.models[0].id.clone();
        assert!(!saved_id.is_empty());
        assert_eq!(config.read().await.models.len(), 1);

        // list 显示激活标记为空
        let out = tool
            .call(ManageModelArgs {
                action: ModelAction::List,
                model: None,
                id: None,
            })
            .await
            .unwrap();
        assert!(out.contains(&saved_id));

        // activate chat
        let out = tool
            .call(ManageModelArgs {
                action: ModelAction::Activate,
                model: None,
                id: Some(saved_id.clone()),
            })
            .await
            .unwrap();
        assert!(out.contains("已激活对话模型"));
        assert_eq!(config.read().await.active_model_id.as_deref(), Some(saved_id.as_str()));

        // 更新（save 同 id）
        m.id = saved_id.clone();
        m.label = "改名了".to_string();
        let out = tool
            .call(ManageModelArgs {
                action: ModelAction::Save,
                model: Some(m),
                id: None,
            })
            .await
            .unwrap();
        assert!(out.contains("已更新模型"));
        assert_eq!(config.read().await.models[0].label, "改名了");

        // delete
        let out = tool
            .call(ManageModelArgs {
                action: ModelAction::Delete,
                model: None,
                id: Some(saved_id),
            })
            .await
            .unwrap();
        assert!(out.contains("已删除模型"));
        assert!(config.read().await.models.is_empty());
        assert!(config.read().await.active_model_id.is_none());
    }

    #[tokio::test]
    async fn activate_missing_model_errors() {
        let (handle, _) = make_handle();
        let tool = ManageModelTool::new(handle);
        let r = tool
            .call(ManageModelArgs {
                action: ModelAction::Activate,
                model: None,
                id: Some("nope".to_string()),
            })
            .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("不存在"));
    }
}
