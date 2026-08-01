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

mod enable;
mod get_detail;
mod install_clawhub;
mod list_installed;
mod search_clawhub;
mod uninstall;

pub use enable::{EnableSkillArgs, EnableSkillError, EnableSkillTool};
pub use get_detail::{GetSkillDetailArgs, GetSkillDetailError, GetSkillDetailTool};
pub use install_clawhub::{
    InstallClawHubSkillArgs, InstallClawHubSkillError, InstallClawHubSkillTool,
};
pub use list_installed::{
    ListInstalledSkillsArgs, ListInstalledSkillsError, ListInstalledSkillsTool,
};
pub use search_clawhub::{
    SearchClawHubSkillsArgs, SearchClawHubSkillsError, SearchClawHubSkillsTool,
};
pub use uninstall::{UninstallSkillArgs, UninstallSkillError, UninstallSkillTool};

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前 Unix 毫秒时间戳；失败回退为 0
#[inline]
pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 截断 id 用于显示（取前 8 字符，UTF-8 边界安全）
#[inline]
pub(super) fn short_id(id: &str) -> String {
    if id.len() <= 8 {
        id.to_string()
    } else {
        id[..id.ceil_char_boundary(8)].to_string()
    }
}

// 测试通过 `super::*` 引用 `Arc` / `RwLock` / `SkillIndex` / `SkillStore` /
// `ConversationStore` / `ClawHubClient` / `Tool` / `resolve_preamble`，
// 仅在测试构建下导入以避免未使用警告
#[cfg(test)]
use {
    effisuite_core::{ClawHubClient, ConversationStore, SkillIndex, SkillStore},
    rig_core::tool::Tool,
    std::sync::Arc,
    tokio::sync::RwLock,
};

// 测试模块通过 `super::*` 引用 `resolve_preamble`（仅在测试构建下需要）
#[cfg(test)]
use enable::resolve_preamble;

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
        let store = SkillStore::new(tmp_path()).unwrap();
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
        let store = SkillStore::new(tmp_path()).unwrap();
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
        let store = SkillStore::new(tmp_path()).unwrap();
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
        let store = SkillStore::new(tmp_path()).unwrap();
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
        let client = ClawHubClient::new();
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
        let client = ClawHubClient::new();
        let store = SkillStore::new(tmp_path()).unwrap();
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
        let store = SkillStore::new(tmp_path()).unwrap();
        let idx = Arc::new(SkillIndex::new());
        let tool = UninstallSkillTool::new(store, Arc::clone(&idx));

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
        let store = SkillStore::new(&store_dir).unwrap();
        let idx = Arc::new(SkillIndex::new());

        let skill = make_skill("abcdef-1234", "Custom", "desc", "");
        store.save(&skill).await.unwrap();
        idx.rebuild_from_store(&store).await.unwrap();

        let tool = UninstallSkillTool::new(store.clone(), Arc::clone(&idx));
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
