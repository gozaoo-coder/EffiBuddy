//! Tauri 命令模块集合：按功能域拆分为子模块，统一 re-export 供 `invoke_handler!` 引用。
//!
//! 子模块划分：
//! - `general`：通用命令（greet / get_agent_backend）
//! - `config`：配置管理（get_config / set_config / set_theme）
//! - `models`：Provider 预设、可使用模型管理、图像生成
//! - `models_remote`：远程模型列表拉取（智能 URL 拼接 / 多态响应解析）
//! - `conversations`：会话 CRUD 与搜索
//! - `memory`：RAG 记忆检索、永久记忆管理、上下文预览
//! - `compression`：消息压缩（非流式 + 流式）
//! - `chat`：聊天（非流式 + 流式）与流式事件 payload
//! - `p2p`：设备扫描与配对
//! - `files`：文件 / 图片选择与读取
//! - `skills`：技能管理与会话工作区
//! - `clawhub`：ClawHub 浏览 / 安装（Skills & Plugins）
//! - `plugins`：插件贡献注册与插件配置（manifest / 生命周期 / appdata 配置）
//! - `scheduled_tasks`：定时任务管理
//! - `asr`：语音转写（流式录音 / 文件转写 / 记录管理 / 摘要 RAG / 配置）

mod asr;
mod chat;
mod clawhub;
mod compression;
mod config;
mod conversations;
mod files;
mod general;
mod git_context;
mod memory;
mod models;
mod models_remote;
pub(crate) mod p2p;
pub(crate) mod plugins;
pub(crate) mod pool;
pub(crate) mod scheduled_tasks;
mod shell_sessions;
mod skills;
mod snapshot;
mod todo_tree;
mod versions;

pub(crate) use asr::*;
pub(crate) use chat::*;
pub(crate) use clawhub::*;
pub(crate) use compression::*;
pub(crate) use config::*;
pub(crate) use conversations::*;
pub(crate) use files::*;
pub(crate) use general::*;
pub(crate) use git_context::*;
pub(crate) use memory::*;
pub(crate) use models::*;
pub(crate) use models_remote::*;
pub(crate) use p2p::*;
pub(crate) use plugins::*;
pub(crate) use pool::*;
pub(crate) use scheduled_tasks::*;
pub(crate) use shell_sessions::*;
pub(crate) use skills::*;
pub(crate) use snapshot::*;
pub(crate) use todo_tree::*;
pub(crate) use versions::*;
