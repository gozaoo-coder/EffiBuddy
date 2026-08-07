//! 外部技能目录扫描（兼容 npx skills / OpenClaw / Claude Code 安装的技能）
//!
//! EffiSuite 官方技能库位于 `appdata/skills`（每个技能一个 `<id>.json`）。
//! 为了兼容社区生态（`npx skills add ...` 装到 `~/.agents/skills`、OpenClaw
//! 的 `~/.openclaw/skills` 等目录型技能，每个技能一个目录、内含 `SKILL.md`），
//! 提供**只读扫描**能力：
//!
//! - 把每个含 `SKILL.md` 的直接子目录解析为一个 [`Skill`]（`source="directory"`）
//! - `working_dir` 指向技能目录，agent 可访问其资源；`preamble` 为 SKILL.md 正文
//! - 多个根目录指向同一批技能（如 openclaw 符号链接到 .agents）时按 canonical 路径去重
//! - 只读：绝不写入 / 修改外部目录，删除外部技能由调用方决定（默认禁止）
//!
//! 根目录列表由上层（tauriFront 的 `paths::external_skills_roots`）传入，
//! 本模块保持零外部依赖（仅 tokio + std），不引入 dirs 等 crate。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{clawhub::parse_skill_md, Skill};

/// 扫描一组外部技能根目录，返回去重后的 Skill 列表（不含内置技能）。
///
/// - 根目录不存在 / 不可读：静默跳过
/// - 子项必须是目录（跟随符号链接，兼容 openclaw 指向 .agents 的链接）
/// - 目录内需存在 `SKILL.md`（大小写均可），否则跳过
/// - 结果顺序不保证，调用方按 `created_at` 自行排序
pub async fn scan_external_skills(roots: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        let mut entries = match tokio::fs::read_dir(root).await {
            Ok(e) => e,
            Err(_) => continue, // 根目录不存在或不可读：跳过
        };
        let root_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("external")
            .to_string();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            // 必须是目录（跟随符号链接）
            if !path.is_dir() {
                continue;
            }
            // canonical 去重：同一批技能被多个根引用时只收录一次
            let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(canon) {
                continue;
            }
            let Some(slug) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(skill_md_path) = find_skill_md(&path) else {
                continue;
            };
            let content = match tokio::fs::read_to_string(&skill_md_path).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            let parsed = parse_skill_md(&content);
            let created_at = tokio::fs::metadata(&path)
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let name = if parsed.name.is_empty() {
                slug.to_string()
            } else {
                parsed.name.clone()
            };
            let description = if parsed.description.is_empty() {
                slug.to_string()
            } else {
                parsed.description.clone()
            };
            out.push(Skill {
                id: slug.to_string(),
                name,
                description,
                preamble: parsed.body,
                tools: Vec::new(),
                working_dir: Some(path.to_string_lossy().into_owned()),
                source: Some("directory".to_string()),
                source_slug: Some(slug.to_string()),
                source_owner: Some(root_name.clone()),
                source_version: if parsed.version.is_empty() {
                    None
                } else {
                    Some(parsed.version)
                },
                created_at,
                builtin: false,
            });
        }
    }
    out
}

/// 在技能目录中定位 `SKILL.md`（优先标准大写，回退小写）。
fn find_skill_md(dir: &Path) -> Option<PathBuf> {
    for name in ["SKILL.md", "skill.md"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "effisuite-external-skill-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn scans_skill_md_directories() {
        let root = tmp_dir();
        let skill_dir = root.join("weather");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Weather\ndescription: Get current weather\nversion: 1.0.0\n---\n# Weather\ncurl wttr.in\n",
        )
        .unwrap();
        // 无 SKILL.md 的目录应被跳过
        std::fs::create_dir_all(root.join("no-skill")).unwrap();

        let skills = scan_external_skills(&[root.clone()]).await;
        assert_eq!(skills.len(), 1);
        let s = &skills[0];
        assert_eq!(s.id, "weather");
        assert_eq!(s.name, "Weather");
        assert_eq!(s.description, "Get current weather");
        assert_eq!(s.source.as_deref(), Some("directory"));
        assert_eq!(s.working_dir.as_deref(), Some(skill_dir.to_str().unwrap()));
        assert!(s.preamble.contains("curl wttr.in"));
        assert!(!s.builtin);
        assert_eq!(s.source_version.as_deref(), Some("1.0.0"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn dedupes_symlinked_roots() {
        let root_a = tmp_dir();
        let root_b = tmp_dir();
        let skill_dir = root_a.join("translate");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Translate\ndescription: Translate text\n---\nBody",
        )
        .unwrap();
        // 第二个根指向同一批技能（模拟 openclaw 符号链接）
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(root_b.join("translate"))
                .arg(&skill_dir)
                .status();
        }
        #[cfg(not(windows))]
        {
            let _ = std::os::unix::fs::symlink(&skill_dir, root_b.join("translate"));
        }

        let skills = scan_external_skills(&[root_a.clone(), root_b.clone()]).await;
        // 同一技能只收录一次（canonical 路径去重）
        let count = skills.iter().filter(|s| s.id == "translate").count();
        assert_eq!(count, 1, "符号链接根应去重，实际收录 {count} 次");

        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    #[tokio::test]
    async fn missing_root_is_ignored() {
        let skills = scan_external_skills(&[tmp_dir().join("nope")]).await;
        assert!(skills.is_empty());
    }
}
