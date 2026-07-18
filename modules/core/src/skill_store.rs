//! 技能（Skill）持久化存储
//!
//! 基于 JSON 文件的简单持久化方案：每个用户技能一个文件，
//! 存放在 `appdata/skills/<id>.json`。内置技能（agent-reach /
//! browser-act）不落盘，由 `list_builtin` / `get` 透明返回。
//!
//! 设计要点与 [`ConversationStore`] 一致：
//! - 读多写少：`RwLock` 允许多读
//! - IO 在锁外完成
//! - `list_all` 用 `with_capacity` 预分配
//! - `get` 先查内置再查磁盘，O(1) 命中内置

use std::path::{Path, PathBuf};

use tokio::sync::RwLock;

use crate::{CoreError, Result, Skill};

/// 内置技能 id：agent-reach
pub const BUILTIN_AGENT_REACH: &str = "agent-reach";
/// 内置技能 id：browser-act
pub const BUILTIN_BROWSER_ACT: &str = "browser-act";

/// 技能存储，线程安全可廉价 clone（内部 RwLock + Arc）
#[derive(Clone)]
pub struct SkillStore {
    root: PathBuf,
    _lock: std::sync::Arc<RwLock<()>>,
}

impl SkillStore {
    /// 创建存储，root 不存在时自动创建
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(CoreError::Io)?;
        Ok(Self {
            root,
            _lock: std::sync::Arc::new(RwLock::new(())),
        })
    }

    /// 技能文件路径：`<root>/<id>.json`
    #[inline]
    fn path_for(&self, id: &str) -> PathBuf {
        // 防止 id 含路径分隔符导致越权访问：仅取文件名部分
        let safe = Path::new(id)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(id));
        self.root.join(safe).with_extension("json")
    }

    /// 返回预置内置技能列表（agent-reach / browser-act）。
    /// `created_at` 置 0，`builtin` 置 true。
    pub fn list_builtin() -> Vec<Skill> {
        vec![
            Skill {
                id: BUILTIN_AGENT_REACH.to_string(),
                name: "Agent Reach".to_string(),
                description: "使用 agent-reach 工具访问互联网并搜索内容".to_string(),
                preamble: "你可以使用 agent-reach 工具访问互联网。执行 `agent-reach doctor` 检查状态，`agent-reach install --channels=all` 安装渠道。用 opencli/twitter/yt-dlp 等命令搜索内容。".to_string(),
                tools: vec!["shell".to_string(), "web_fetch".to_string()],
                working_dir: None,
                created_at: 0,
                builtin: true,
                source: None,
                source_slug: None,
                source_owner: None,
                source_version: None,
            },
            Skill {
                id: BUILTIN_BROWSER_ACT.to_string(),
                name: "Browser Act".to_string(),
                description: "使用 browser-act 进行浏览器自动化".to_string(),
                preamble: "你可以使用 browser-act 进行浏览器自动化。执行 `browser-act get-skills core` 获取能力，`browser-act fetch URL` 抓取页面。".to_string(),
                tools: vec!["shell".to_string(), "web_fetch".to_string()],
                working_dir: None,
                created_at: 0,
                builtin: true,
                source: None,
                source_slug: None,
                source_owner: None,
                source_version: None,
            },
        ]
    }

    /// 列出用户自定义技能（仅磁盘文件，不含内置）。
    ///
    /// 仅扫描 `*.json` 文件，跳过子目录（ClawHub 技能解压目录使用子文件夹存放资源）。
    pub async fn list_user(&self) -> Result<Vec<Skill>> {
        let mut entries = tokio::fs::read_dir(&self.root).await.map_err(CoreError::Io)?;
        let mut out = Vec::with_capacity(8);
        while let Some(entry) = entries.next_entry().await.map_err(CoreError::Io)? {
            let path = entry.path();
            // 仅处理 .json 文件，跳过目录与非 json 文件
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let skill: Skill = match serde_json::from_slice(&bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };
            out.push(skill);
        }
        // 按 created_at 降序，新的在前
        out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(out)
    }

    /// 列出全部技能：内置在前，用户自定义在后。
    pub async fn list_all(&self) -> Result<Vec<Skill>> {
        let mut out = Self::list_builtin();
        out.extend(self.list_user().await?);
        Ok(out)
    }

    /// 加载单个技能：先查内置（按 id），再查磁盘。
    pub async fn get(&self, id: &str) -> Result<Option<Skill>> {
        if let Some(b) = Self::list_builtin().into_iter().find(|s| s.id == id) {
            return Ok(Some(b));
        }
        let path = self.path_for(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
        let skill: Skill = serde_json::from_slice(&bytes).map_err(CoreError::Serde)?;
        Ok(Some(skill))
    }

    /// 按 ClawHub slug 查找已安装技能（用于检测是否已安装 / 跳过重复安装）。
    ///
    /// 遍历用户技能列表，匹配 `source_slug == slug`。O(n) 但 n 通常很小。
    pub async fn find_by_clawhub_slug(&self, slug: &str) -> Result<Option<Skill>> {
        let user_skills = self.list_user().await?;
        Ok(user_skills
            .into_iter()
            .find(|s| s.source.as_deref() == Some("clawhub") && s.source_slug.as_deref() == Some(slug)))
    }

    /// 保存（或覆盖）一个用户技能。
    /// 内置技能 id 落盘无意义，但允许写入以支持自定义同名覆盖场景。
    pub async fn save(&self, skill: &Skill) -> Result<()> {
        let path = self.path_for(&skill.id);
        let bytes = serde_json::to_vec(skill).map_err(CoreError::Serde)?;
        tokio::fs::write(&path, bytes).await.map_err(CoreError::Io)?;
        Ok(())
    }

    /// 删除指定技能文件；内置技能在磁盘上不存在，直接返回 Ok。
    ///
    /// 若技能是 ClawHub 安装技能（有同名解压目录），同时递归删除目录。
    pub async fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id);
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .map_err(CoreError::Io)?;
        }
        // 删除 ClawHub 技能解压目录（若存在）：<root>/<id>/
        let dir = self.root.join(id);
        if dir.is_dir() {
            if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
                tracing::warn!(error = %e, dir = ?dir, "删除 ClawHub 技能目录失败（忽略）");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("effisuite-skill-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn builtin_listed_and_gettable() {
        let store = SkillStore::new(tmp_dir()).unwrap();
        let all = store.list_all().await.unwrap();
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&BUILTIN_AGENT_REACH));
        assert!(ids.contains(&BUILTIN_BROWSER_ACT));

        let reach = store.get(BUILTIN_AGENT_REACH).await.unwrap().unwrap();
        assert!(reach.builtin);
        assert!(!reach.preamble.is_empty());
    }

    #[tokio::test]
    async fn save_get_delete_user_skill() {
        let store = SkillStore::new(tmp_dir()).unwrap();
        let skill = Skill {
            id: "custom-1".to_string(),
            name: "Custom".to_string(),
            description: "d".to_string(),
            preamble: "p".to_string(),
            tools: vec!["shell".to_string()],
            working_dir: None,
            created_at: 1,
            builtin: false,
            source: None,
            source_slug: None,
            source_owner: None,
            source_version: None,
        };
        store.save(&skill).await.unwrap();
        let got = store.get("custom-1").await.unwrap().unwrap();
        assert_eq!(got.name, "Custom");
        assert!(!got.builtin);

        store.delete("custom-1").await.unwrap();
        assert!(store.get("custom-1").await.unwrap().is_none());
    }
}
