//! 技能管理工具集：让 LLM 自主发现 / 启用 / 搜索 / 安装技能
//!
//! 5 个独立工具（rig `Tool` trait），共享 `SkillIndex` / `SkillStore` /
//! `ClawHubClient` / `ConversationStore` 句柄：
//!
//! - [`ListInstalledSkillsTool`]：列出所有已安装技能（id/name/description/builtin）
//! - [`GetSkillDetailTool`]：按 id 获取技能完整 preamble + working_dir，
//!   让 agent 据此判断"如何使用这个技能"以及访问技能携带的资源文件
//! - [`EnableSkillTool`]：把指定技能的 preamble 注入到当前会话上下文
//!   （作为 System 消息追加到对话历史，替代旧 apply_skill 命令）
//! - [`SearchClawHubSkillsTool`]：从 ClawHub 远程搜索未安装的技能
//! - [`InstallClawHubSkillTool`]：从 ClawHub 下载并安装技能
//!
//! # 设计理念
//!
//! 移除用户手动点击"应用技能"的步骤，改为：
//! 1. RigAgent 在 build_context_parts 中自动检索 Top-K 相关技能摘要
//!    注入到 `[可用技能]` 段（agent 知道"我能用什么"）
//! 2. agent 通过 `get_skill_detail` 按需深入了解某个技能
//! 3. agent 通过 `enable_skill` 把技能 preamble 注入会话上下文
//! 4. agent 通过 `search_clawhub_skills` / `install_clawhub_skill`
//!    主动扩展能力（当本地无匹配技能时）
//!
//! # 设计要点（对齐 user_rules）
//!
//! - 工具本身无状态，所有数据在共享 `Arc<...>` 中
//! - IO 在锁外完成（SkillStore / ClawHubClient 都是异步 IO，工具内不持锁）
//! - 返回纯文本，流式友好；错误以 `"Error: ..."` 前缀标记

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use effisuite_core::{
    ClawHubClient, ConversationStore, Message, Role, Skill, SkillIndex, SkillStore,
};
use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

/// 当前 Unix 毫秒时间戳；失败回退为 0
#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 截断 id 用于显示（取前 8 字符，UTF-8 边界安全）
#[inline]
fn short_id(id: &str) -> String {
    if id.len() <= 8 {
        id.to_string()
    } else {
        id[..id.ceil_char_boundary(8)].to_string()
    }
}

// =========================================================
// ListInstalledSkillsTool：列出所有已安装技能
// =========================================================

/// 无参数（rig 要求 Args 类型，用空 struct + 自定义 deserialize）
#[derive(Deserialize, Default)]
pub struct ListInstalledSkillsArgs {}

/// 列表工具错误
#[derive(Debug, thiserror::Error)]
#[error("list installed skills error: {0}")]
pub struct ListInstalledSkillsError(String);

/// 列出所有已安装技能工具
///
/// 持有 `SkillIndex` 共享句柄（与 RigAgent 共享同一份索引快照）。
/// 走索引读路径，零 IO；技能增删后由 Tauri 命令层 rebuild 索引。
pub struct ListInstalledSkillsTool {
    index: Arc<SkillIndex>,
}

impl ListInstalledSkillsTool {
    pub fn new(index: Arc<SkillIndex>) -> Self {
        Self { index }
    }
}

impl Tool for ListInstalledSkillsTool {
    const NAME: &'static str = "list_installed_skills";

    type Error = ListInstalledSkillsError;
    type Args = ListInstalledSkillsArgs;
    type Output = String;

    fn description(&self) -> String {
        "列出当前已安装的全部技能（id/名称/简介/是否内置）。\
         在需要了解本地有哪些可用能力、或准备深入使用某个技能前调用。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let list = self.index.list_all().await;
        if list.is_empty() {
            return Ok("当前没有任何已安装技能。可调用 search_clawhub_skills 从 ClawHub 查找并安装。".to_string());
        }
        let mut out = String::with_capacity(list.len() * 96);
        out.push_str(&format!("当前共 {} 个已安装技能：\n", list.len()));
        for (i, s) in list.iter().enumerate() {
            let tag = if s.builtin { "[内置]" } else { "" };
            out.push_str(&format!(
                "{}. (id={}) {}{} — {}\n",
                i + 1,
                short_id(&s.id),
                tag,
                s.name,
                s.description
            ));
        }
        out.push_str("\n提示：调用 get_skill_detail(id) 可获取技能完整说明；调用 enable_skill(id) 把技能指令注入会话上下文。");
        Ok(out)
    }
}

// =========================================================
// GetSkillDetailTool：按 id 获取技能完整 preamble + working_dir
// =========================================================

#[derive(Deserialize)]
pub struct GetSkillDetailArgs {
    /// 技能 id（完整或前 8 字符前缀）
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("get skill detail error: {0}")]
pub struct GetSkillDetailError(String);

/// 获取技能详情工具
///
/// 持有 `SkillStore` 共享句柄，按 id 读取完整 Skill（含 preamble / working_dir）。
/// agent 据此判断"如何使用这个技能"，并可用 read_file / list_files /
/// shell 工具访问 working_dir 下的技能资源（脚本、配置等）。
pub struct GetSkillDetailTool {
    store: Arc<SkillStore>,
}

impl GetSkillDetailTool {
    pub fn new(store: Arc<SkillStore>) -> Self {
        Self { store }
    }
}

impl Tool for GetSkillDetailTool {
    const NAME: &'static str = "get_skill_detail";

    type Error = GetSkillDetailError;
    type Args = GetSkillDetailArgs;
    type Output = String;

    fn description(&self) -> String {
        "按 id 获取已安装技能的完整说明（preamble 全文 + 工作目录）。\
         在准备使用某技能、需要了解技能完整指令、或访问技能附带资源文件时调用。\
         id 支持前 8 字符前缀匹配。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "技能 id（完整或前 8 字符前缀）"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.id.trim();
        if target.is_empty() {
            return Err(GetSkillDetailError("id 不能为空".to_string()));
        }

        // 先精确匹配；不中再前缀匹配（与 DeletePinnedMemoryTool 一致）
        let skill = match self.store.get(target).await {
            Ok(Some(s)) => Some(s),
            _ => {
                let all = self
                    .store
                    .list_all()
                    .await
                    .map_err(|e| GetSkillDetailError(e.to_string()))?;
                // 前缀匹配优先；若前缀匹配多条则退化为精确匹配
                let prefix_match: Vec<&Skill> =
                    all.iter().filter(|s| s.id.starts_with(target)).collect();
                match prefix_match.len() {
                    1 => Some(prefix_match[0].clone()),
                    0 => all.into_iter().find(|s| s.id == target),
                    _ => {
                        // 多条前缀匹配：返回 None 让上层走"未找到"分支，
                        // 引导用户提供更长 id
                        None
                    }
                }
            }
        };

        let Some(skill) = skill else {
            return Ok(format!(
                "未找到 id 包含「{}」的技能。可调用 list_installed_skills 查看全部已安装技能。",
                target
            ));
        };

        let mut out = String::with_capacity(skill.preamble.len() + 256);
        out.push_str(&format!("技能：{}（id={}）\n", skill.name, short_id(&skill.id)));
        out.push_str(&format!("简介：{}\n", skill.description));
        if skill.builtin {
            out.push_str("类型：内置\n");
        } else if skill.source.as_deref() == Some("clawhub") {
            out.push_str("来源：ClawHub\n");
        }
        if let Some(wd) = skill.working_dir.as_deref() {
            out.push_str(&format!("工作目录：{}\n", wd));
            out.push_str("（可用 read_file / list_files / shell 工具访问此目录下的资源）\n");
        }
        out.push_str("\n--- 技能指令（preamble）---\n");
        if skill.preamble.is_empty() {
            out.push_str("（preamble 为空，技能无显式指令；可能仅提供工作目录资源）\n");
        } else {
            out.push_str(&skill.preamble);
            if !skill.preamble.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("\n--- 结束 ---\n");
        out.push_str("如需把此技能指令注入会话上下文以便后续对话遵循，调用 enable_skill(id)。");
        Ok(out)
    }
}

// =========================================================
// EnableSkillTool：把技能 preamble 注入当前会话上下文
// =========================================================

#[derive(Deserialize)]
pub struct EnableSkillArgs {
    /// 技能 id（完整或前 8 字符前缀）
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("enable skill error: {0}")]
pub struct EnableSkillError(String);

/// 启用技能工具
///
/// 持有：
/// - `SkillStore`：读取技能 preamble（ClawHub 技能 preamble 为 SKILL.md 正文）
/// - `ConversationStore`：把 preamble 作为 System 消息追加到当前会话历史
/// - `current_conversation_id`：当前会话 id 句柄
///
/// 替代旧 apply_skill 命令；用户不再需要手动点击应用，由 agent 主动调用。
/// 注入后，后续对话的 prompt 会包含此技能指令，agent 据此"使用"技能。
pub struct EnableSkillTool {
    store: Arc<SkillStore>,
    conversation_store: Arc<ConversationStore>,
    current_conversation_id: Arc<RwLock<Option<String>>>,
}

impl EnableSkillTool {
    pub fn new(
        store: Arc<SkillStore>,
        conversation_store: Arc<ConversationStore>,
        current_conversation_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            store,
            conversation_store,
            current_conversation_id,
        }
    }
}

impl Tool for EnableSkillTool {
    const NAME: &'static str = "enable_skill";

    type Error = EnableSkillError;
    type Args = EnableSkillArgs;
    type Output = String;

    fn description(&self) -> String {
        "启用指定技能：把技能指令（preamble）注入当前会话上下文。\
         注入后，后续对话会遵循此技能的指令行事。\
         在需要使用某技能、或用户要求启用某技能时调用。\
         id 支持前 8 字符前缀匹配。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "要启用的技能 id（完整或前 8 字符前缀）"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.id.trim();
        if target.is_empty() {
            return Err(EnableSkillError("id 不能为空".to_string()));
        }

        // 解析当前会话 id
        let conv_id = self.current_conversation_id.read().await.clone();
        let Some(conv_id) = conv_id else {
            return Err(EnableSkillError("当前没有活动会话，无法启用技能".to_string()));
        };

        // 加载技能（先精确，后前缀）
        let skill = match self.store.get(target).await {
            Ok(Some(s)) => Some(s),
            _ => {
                let all = self
                    .store
                    .list_all()
                    .await
                    .map_err(|e| EnableSkillError(e.to_string()))?;
                all.into_iter().find(|s| s.id.starts_with(target))
            }
        };
        let Some(skill) = skill else {
            return Ok(format!(
                "未找到 id 包含「{}」的技能。可调用 list_installed_skills 查看全部已安装技能。",
                target
            ));
        };

        // 工作目录注入：若技能配置了 working_dir 且会话级 working_dir 未设置，写入会话级
        // （与旧 apply_skill 行为一致，agent 据此访问技能资源）
        if let Some(skill_wd) = skill.working_dir.clone() {
            let conv = self
                .conversation_store
                .load(&conv_id)
                .await
                .map_err(|e| EnableSkillError(e.to_string()))?;
            let need_set = conv
                .as_ref()
                .map(|c| c.working_dir.is_none())
                .unwrap_or(true);
            if need_set {
                self.conversation_store
                    .set_working_dir(&conv_id, Some(skill_wd))
                    .await
                    .map_err(|e| EnableSkillError(e.to_string()))?;
            }
        }

        // 解析 preamble：优先持久化值；为空且 working_dir 含 SKILL.md 时回读
        // （兼容此修复前安装的 ClawHub 技能 + 支持外部编辑 SKILL.md 热更新）
        let preamble = resolve_preamble(&skill).await;
        if preamble.is_empty() {
            return Ok(format!(
                "技能「{}」无可用指令（preamble 为空），仅工作目录已注入会话。",
                skill.name
            ));
        }

        // 把 preamble 作为 System 消息追加到会话历史
        let sys_msg = Message::new(
            uuid::Uuid::new_v4().to_string(),
            Role::System,
            preamble,
            now_ms(),
        );
        self.conversation_store
            .append_message(&conv_id, sys_msg, now_ms())
            .await
            .map_err(|e| EnableSkillError(e.to_string()))?;

        Ok(format!(
            "已启用技能「{}」（id={}）。技能指令已注入会话上下文，后续对话将遵循此指令。{}",
            skill.name,
            short_id(&skill.id),
            if skill.working_dir.is_some() {
                "\n工作目录也已注入，可用 read_file / list_files / shell 访问技能资源。"
            } else {
                ""
            }
        ))
    }
}

/// 解析技能最终要注入到会话的 preamble 文本。
///
/// 优先级：
/// 1. `skill.preamble` 非空 → 直接返回
/// 2. `skill.working_dir` 下存在 `SKILL.md` → 现读并返回其正文
/// 3. 否则返回空串
async fn resolve_preamble(skill: &Skill) -> String {
    if !skill.preamble.is_empty() {
        return skill.preamble.clone();
    }
    let Some(wd) = skill.working_dir.as_deref() else {
        return String::new();
    };
    let skill_md = std::path::Path::new(wd).join("SKILL.md");
    let content = match tokio::fs::read_to_string(&skill_md).await {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    effisuite_core::clawhub::parse_skill_md(&content).body
}

// =========================================================
// SearchClawHubSkillsTool：从 ClawHub 搜索技能
// =========================================================

#[derive(Deserialize)]
pub struct SearchClawHubSkillsArgs {
    /// 搜索关键词
    pub query: String,
    /// 最多返回多少条结果，默认 10
    #[serde(default = "default_search_limit")]
    pub limit: u32,
}

fn default_search_limit() -> u32 {
    10
}

#[derive(Debug, thiserror::Error)]
#[error("search clawhub skills error: {0}")]
pub struct SearchClawHubSkillsError(String);

/// 从 ClawHub 搜索技能工具
///
/// 持有 `ClawHubClient` 共享句柄。当本地无匹配技能时，agent 据此
/// 从 ClawHub 远程搜索，找到合适 slug 后调 install_clawhub_skill 安装。
pub struct SearchClawHubSkillsTool {
    client: Arc<ClawHubClient>,
}

impl SearchClawHubSkillsTool {
    pub fn new(client: Arc<ClawHubClient>) -> Self {
        Self { client }
    }
}

impl Tool for SearchClawHubSkillsTool {
    const NAME: &'static str = "search_clawhub_skills";

    type Error = SearchClawHubSkillsError;
    type Args = SearchClawHubSkillsArgs;
    type Output = String;

    fn description(&self) -> String {
        "从 ClawHub 远程技能市场搜索技能（不限于本地已安装）。\
         当用户需要的能力本地已安装技能都无法满足时调用。\
         找到合适技能后，用返回的 slug 调用 install_clawhub_skill 安装。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索关键词，如 'weather' / '翻译' / 'code review'"
                },
                "limit": {
                    "type": "integer",
                    "description": "最多返回的结果条数，默认 10",
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query = args.query.trim();
        if query.is_empty() {
            return Err(SearchClawHubSkillsError("query 不能为空".to_string()));
        }
        let limit = if args.limit == 0 { 10 } else { args.limit };

        let resp = self
            .client
            .search_skills(query, Some(limit))
            .await
            .map_err(|e| SearchClawHubSkillsError(e.to_string()))?;

        if resp.results.is_empty() {
            return Ok(format!("未在 ClawHub 找到与「{}」相关的技能。", query));
        }

        let mut out = String::with_capacity(resp.results.len() * 128);
        out.push_str(&format!("在 ClawHub 找到 {} 个相关技能：\n", resp.results.len()));
        for (i, r) in resp.results.iter().enumerate() {
            let slug = r.slug.as_deref().unwrap_or("(无 slug)");
            let name = r.display_name.as_deref().unwrap_or("(未命名)");
            let summary = r.summary.as_deref().unwrap_or("(无简介)");
            let owner = r
                .owner_handle
                .as_deref()
                .or_else(|| r.owner.as_ref().and_then(|o| o.handle.as_deref()))
                .unwrap_or("(未知)");
            let version = r.version.as_deref().unwrap_or("?");
            out.push_str(&format!(
                "{}. [slug={}] {} (v{}, by {})\n   {}\n",
                i + 1,
                slug,
                name,
                version,
                owner,
                summary
            ));
        }
        out.push_str("\n提示：用 install_clawhub_skill(slug) 安装选定技能。");
        Ok(out)
    }
}

// =========================================================
// InstallClawHubSkillTool：从 ClawHub 下载并安装技能
// =========================================================

#[derive(Deserialize)]
pub struct InstallClawHubSkillArgs {
    /// ClawHub 技能 slug（来自 search_clawhub_skills 结果）
    pub slug: String,
}

#[derive(Debug, thiserror::Error)]
#[error("install clawhub skill error: {0}")]
pub struct InstallClawHubSkillError(String);

/// 从 ClawHub 安装技能工具
///
/// 持有：
/// - `ClawHubClient`：HTTP 下载 + 元数据查询
/// - `SkillStore`：持久化安装的技能
/// - `SkillIndex`：安装后 rebuild 索引，让下一轮 RAG 自动注入看到新技能
/// - `skills_dir`：技能解压根目录
///
/// 安装流程与 Tauri 层 `clawhub_install_skill` 命令一致：
/// 1. 幂等检查（find_by_clawhub_slug）
/// 2. 下载 ZIP（5min 超时）
/// 3. spawn_blocking 解压到 <skills_dir>/<slug>/
/// 4. 解析 SKILL.md frontmatter + 正文
/// 5. 落盘 SkillStore（preamble=SKILL.md 正文）
/// 6. rebuild SkillIndex
pub struct InstallClawHubSkillTool {
    client: Arc<ClawHubClient>,
    store: Arc<SkillStore>,
    index: Arc<SkillIndex>,
    skills_dir: std::path::PathBuf,
}

impl InstallClawHubSkillTool {
    pub fn new(
        client: Arc<ClawHubClient>,
        store: Arc<SkillStore>,
        index: Arc<SkillIndex>,
        skills_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            client,
            store,
            index,
            skills_dir,
        }
    }
}

impl Tool for InstallClawHubSkillTool {
    const NAME: &'static str = "install_clawhub_skill";

    type Error = InstallClawHubSkillError;
    type Args = InstallClawHubSkillArgs;
    type Output = String;

    fn description(&self) -> String {
        "从 ClawHub 下载并安装指定技能。安装后技能立即可用：\
         下一轮对话的 RAG 自动注入会包含它，也可直接调用 enable_skill 启用。\
         slug 可从 search_clawhub_skills 结果获取。安装是幂等的，重复安装返回已存在 id。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "ClawHub 技能 slug（来自 search_clawhub_skills 结果）"
                }
            },
            "required": ["slug"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        use effisuite_core::clawhub::{extract_zip_to, parse_skill_md};

        let slug = args.slug.trim();
        if slug.is_empty() {
            return Err(InstallClawHubSkillError("slug 不能为空".to_string()));
        }

        // 1. 幂等：已安装则直接返回
        if let Some(existing) = self
            .store
            .find_by_clawhub_slug(slug)
            .await
            .map_err(|e| InstallClawHubSkillError(e.to_string()))?
        {
            return Ok(format!(
                "技能「{}」（slug={}）已安装，无需重复安装。可调用 enable_skill(id={}) 启用。",
                existing.name, slug, short_id(&existing.id)
            ));
        }

        // 2. 拉详情获取 owner / latest_version
        let detail = self
            .client
            .get_skill(slug)
            .await
            .map_err(|e| InstallClawHubSkillError(format!("获取技能详情失败: {e}")))?;
        let owner_handle = detail
            .owner
            .as_ref()
            .and_then(|o| o.handle.clone())
            .unwrap_or_default();
        let version = detail
            .latest_version
            .as_ref()
            .map(|v| v.version.clone())
            .unwrap_or_default();

        // 3. 下载 ZIP
        let zip_bytes = self
            .client
            .download_skill_zip(slug, None, None)
            .await
            .map_err(|e| InstallClawHubSkillError(format!("下载技能包失败: {e}")))?;

        // 4. 解压到 <skills_dir>/<slug>/
        let dest_dir = self.skills_dir.join(slug);
        let dest_for_blocking = dest_dir.clone();
        tokio::task::spawn_blocking(move || extract_zip_to(&dest_for_blocking, &zip_bytes))
            .await
            .map_err(|e| InstallClawHubSkillError(format!("解压任务调度失败: {e}")))?
            .map_err(|e| InstallClawHubSkillError(format!("解压失败: {e}")))?;

        // 5. 解析 SKILL.md：提取 frontmatter 字段 + 正文作为 preamble
        let skill_md_path = dest_dir.join("SKILL.md");
        let (name, description, parsed_version, body) =
            match tokio::fs::read_to_string(&skill_md_path).await {
                Ok(content) => {
                    let p = parse_skill_md(&content);
                    (
                        if p.name.is_empty() { slug.to_string() } else { p.name },
                        if p.description.is_empty() {
                            format!("ClawHub 技能: {}", slug)
                        } else {
                            p.description
                        },
                        if p.version.is_empty() { version.clone() } else { p.version },
                        p.body,
                    )
                }
                Err(_) => (
                    slug.to_string(),
                    format!("ClawHub 技能: {}", slug),
                    version.clone(),
                    String::new(),
                ),
            };

        // 6. 落盘 SkillStore
        let skill = Skill {
            id: slug.to_string(),
            name,
            description,
            preamble: body,
            tools: Vec::new(),
            working_dir: Some(dest_dir.to_string_lossy().into_owned()),
            created_at: now_ms(),
            builtin: false,
            source: Some("clawhub".to_string()),
            source_slug: Some(slug.to_string()),
            source_owner: if owner_handle.is_empty() {
                None
            } else {
                Some(owner_handle)
            },
            source_version: if parsed_version.is_empty() {
                None
            } else {
                Some(parsed_version)
            },
        };
        self.store
            .save(&skill)
            .await
            .map_err(|e| InstallClawHubSkillError(e.to_string()))?;

        // 7. rebuild SkillIndex，让下一轮 RAG 自动注入看到新技能
        self.index
            .rebuild_from_store(&self.store)
            .await
            .map_err(|e| InstallClawHubSkillError(e.to_string()))?;

        Ok(format!(
            "已安装技能「{}」（slug={}，id={}）。\
             下一轮对话的 RAG 自动注入会包含它，也可直接调用 enable_skill(id={}) 启用。",
            skill.name,
            slug,
            short_id(&skill.id),
            short_id(&skill.id)
        ))
    }
}

// =========================================================
// UninstallSkillTool：卸载已安装技能
// =========================================================

#[derive(Deserialize)]
pub struct UninstallSkillArgs {
    /// 技能 id（完整或前 8 字符前缀）
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("uninstall skill error: {0}")]
pub struct UninstallSkillError(String);

/// 卸载技能工具
///
/// 持有 `SkillStore` 与 `SkillIndex` 共享句柄。先定位技能，禁止删除内置技能，
/// 删除后重建索引，让下一轮 RAG 自动注入与 list_installed_skills 工具看到最新数据。
pub struct UninstallSkillTool {
    store: Arc<SkillStore>,
    index: Arc<SkillIndex>,
}

impl UninstallSkillTool {
    pub fn new(store: Arc<SkillStore>, index: Arc<SkillIndex>) -> Self {
        Self { store, index }
    }
}

impl Tool for UninstallSkillTool {
    const NAME: &'static str = "uninstall_skill";

    type Error = UninstallSkillError;
    type Args = UninstallSkillArgs;
    type Output = String;

    fn description(&self) -> String {
        "卸载已安装的技能。内置技能不可卸载；id 支持前 8 字符前缀匹配。\
         卸载后索引会自动重建，agent 将不再看到此技能。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "要卸载的技能 id（完整或前 8 字符前缀）"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.id.trim();
        if target.is_empty() {
            return Err(UninstallSkillError("id 不能为空".to_string()));
        }

        // 定位技能：先精确匹配，再前缀匹配
        let skill = match self.store.get(target).await {
            Ok(Some(s)) => Some(s),
            _ => {
                let all = self
                    .store
                    .list_all()
                    .await
                    .map_err(|e| UninstallSkillError(e.to_string()))?;
                let prefix_match: Vec<&Skill> =
                    all.iter().filter(|s| s.id.starts_with(target)).collect();
                match prefix_match.len() {
                    1 => Some(prefix_match[0].clone()),
                    0 => all.into_iter().find(|s| s.id == target),
                    _ => None,
                }
            }
        };

        let Some(skill) = skill else {
            return Ok(format!(
                "未找到 id 包含「{}」的技能。可调用 list_installed_skills 查看全部已安装技能。",
                target
            ));
        };

        if skill.builtin {
            return Err(UninstallSkillError(format!(
                "技能「{}」（id={}）是内置技能，不可卸载",
                skill.name,
                short_id(&skill.id)
            )));
        }

        let name = skill.name.clone();
        let short = short_id(&skill.id);

        self.store
            .delete(&skill.id)
            .await
            .map_err(|e| UninstallSkillError(e.to_string()))?;
        self.index
            .rebuild_from_store(&self.store)
            .await
            .map_err(|e| UninstallSkillError(e.to_string()))?;

        Ok(format!(
            "已卸载技能「{}」（id={}）。索引已重建，agent 将不再看到此技能。",
            name, short
        ))
    }
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use effisuite_core::Skill;

    fn tmp_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "effisuite-skill-tool-test-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    fn make_skill(id: &str, name: &str, desc: &str, preamble: &str) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            preamble: preamble.to_string(),
            tools: Vec::new(),
            working_dir: None,
            created_at: 1,
            builtin: false,
            source: None,
            source_slug: None,
            source_owner: None,
            source_version: None,
        }
    }

    /// 带 builtin 标记的 make_skill 变体
    fn make_skill_builtin(id: &str, name: &str, desc: &str) -> Skill {
        Skill {
            builtin: true,
            ..make_skill(id, name, desc, "")
        }
    }

    #[tokio::test]
    async fn list_installed_skills_empty() {
        let idx = Arc::new(SkillIndex::new());
        let tool = ListInstalledSkillsTool::new(idx);
        let out = tool.call(ListInstalledSkillsArgs {}).await.unwrap();
        assert!(out.contains("没有任何已安装技能"));
        assert!(out.contains("search_clawhub_skills"));
    }

    #[tokio::test]
    async fn list_installed_skills_with_entries() {
        let idx = Arc::new(SkillIndex::new());
        idx.rebuild(vec![
            make_skill("weather", "Weather", "forecast", ""),
            make_skill_builtin("builtin-1", "Builtin", "built-in"),
        ])
        .await;
        let tool = ListInstalledSkillsTool::new(idx);
        let out = tool.call(ListInstalledSkillsArgs {}).await.unwrap();
        assert!(out.contains("2 个已安装技能"));
        assert!(out.contains("Weather"));
        assert!(out.contains("[内置]"));
    }

    #[tokio::test]
    async fn get_skill_detail_by_prefix() {
        let store = Arc::new(SkillStore::new(tmp_path()).unwrap());
        store
            .save(&make_skill(
                "abcdef-1234",
                "Test",
                "desc",
                "preamble content",
            ))
            .await
            .unwrap();
        let tool = GetSkillDetailTool::new(store);
        let out = tool
            .call(GetSkillDetailArgs {
                id: "abcdef".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("Test"));
        assert!(out.contains("preamble content"));
    }

    #[tokio::test]
    async fn get_skill_detail_not_found() {
        let store = Arc::new(SkillStore::new(tmp_path()).unwrap());
        let tool = GetSkillDetailTool::new(store);
        let out = tool
            .call(GetSkillDetailArgs {
                id: "nonexistent".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("未找到"));
    }

    #[tokio::test]
    async fn get_skill_detail_rejects_empty_id() {
        let store = Arc::new(SkillStore::new(tmp_path()).unwrap());
        let tool = GetSkillDetailTool::new(store);
        let res = tool
            .call(GetSkillDetailArgs {
                id: "  ".to_string(),
            })
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn enable_skill_no_conversation_returns_error() {
        let store = Arc::new(SkillStore::new(tmp_path()).unwrap());
        let conv_store = Arc::new(ConversationStore::new(tmp_path()).unwrap());
        let cur = Arc::new(RwLock::new(None));
        let tool = EnableSkillTool::new(store, conv_store, cur);
        let res = tool
            .call(EnableSkillArgs {
                id: "any".to_string(),
            })
            .await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("没有活动会话"));
    }

    #[tokio::test]
    async fn resolve_preamble_uses_persisted_value() {
        let skill = make_skill("a", "A", "desc", "persisted preamble");
        let p = resolve_preamble(&skill).await;
        assert_eq!(p, "persisted preamble");
    }

    #[tokio::test]
    async fn resolve_preamble_empty_when_no_working_dir() {
        let skill = make_skill("a", "A", "desc", "");
        let p = resolve_preamble(&skill).await;
        assert!(p.is_empty());
    }

    #[tokio::test]
    async fn search_clawhub_skills_rejects_empty_query() {
        let client = Arc::new(ClawHubClient::new());
        let tool = SearchClawHubSkillsTool::new(client);
        let res = tool
            .call(SearchClawHubSkillsArgs {
                query: "  ".to_string(),
                limit: 5,
            })
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn install_clawhub_skill_rejects_empty_slug() {
        let client = Arc::new(ClawHubClient::new());
        let store = Arc::new(SkillStore::new(tmp_path()).unwrap());
        let idx = Arc::new(SkillIndex::new());
        let tool = InstallClawHubSkillTool::new(client, store, idx, std::env::temp_dir());
        let res = tool
            .call(InstallClawHubSkillArgs {
                slug: "  ".to_string(),
            })
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn uninstall_skill_rejects_builtin() {
        let store = Arc::new(SkillStore::new(tmp_path()).unwrap());
        let idx = Arc::new(SkillIndex::new());
        let tool = UninstallSkillTool::new(Arc::clone(&store), Arc::clone(&idx));

        // 内置技能 agent-reach 无需落盘即可 get 到
        let res = tool
            .call(UninstallSkillArgs {
                id: "agent-reach".to_string(),
            })
            .await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("内置技能"));
    }

    #[tokio::test]
    async fn uninstall_skill_by_prefix_rebuilds_index() {
        let store_dir = tmp_path().parent().unwrap().join(format!(
            "effisuite-skill-uninstall-{}-{}",
            uuid::Uuid::new_v4(),
            "store"
        ));
        std::fs::create_dir_all(&store_dir).unwrap();
        let store = Arc::new(SkillStore::new(&store_dir).unwrap());
        let idx = Arc::new(SkillIndex::new());

        let skill = make_skill("abcdef-1234", "Custom", "desc", "");
        store.save(&skill).await.unwrap();
        idx.rebuild_from_store(&store).await.unwrap();

        let tool = UninstallSkillTool::new(Arc::clone(&store), Arc::clone(&idx));
        let out = tool
            .call(UninstallSkillArgs {
                id: "abcdef".to_string(),
            })
            .await
            .unwrap();
        assert!(out.contains("已卸载技能"));
        assert!(out.contains("Custom"));
        assert!(store.get("abcdef-1234").await.unwrap().is_none());

        // 索引也应同步
        let all = idx.list_all().await;
        assert!(!all.iter().any(|s| s.id == "abcdef-1234"));

        std::fs::remove_dir_all(&store_dir).ok();
    }
}
