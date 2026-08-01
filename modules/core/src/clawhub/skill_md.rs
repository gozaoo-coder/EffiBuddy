/// 解析 `SKILL.md` 的 YAML frontmatter，提取 `name` / `description` / `version`。
///
/// frontmatter 格式：
/// ```yaml
/// ---
/// name: my-skill
/// description: Short summary
/// version: 1.0.0
/// ---
/// ```
///
/// 失败时返回空结构（不阻断安装，仅用作展示）。
/// - `preamble`：整个文件内容（含 frontmatter），保留供需要完整内容的场景使用
/// - `body`：去除 frontmatter 后的正文，作为 LLM 系统消息注入最干净
pub fn parse_skill_md(content: &str) -> ParsedSkillMd {
    // 整个文件作为 preamble（保留完整内容）；默认 body 等于 preamble（无 frontmatter 时两者一致）
    let mut parsed = ParsedSkillMd {
        preamble: content.to_string(),
        body: content.to_string(),
        ..Default::default()
    };

    // 检测 frontmatter：以 `---` 开头
    if !content.starts_with("---") {
        return parsed;
    }
    // 找到结束 `---`
    let after_start = &content[3..];
    let end = match after_start.find("\n---") {
        Some(idx) => idx,
        None => return parsed,
    };
    let yaml_block = &after_start[..end];
    // 简单按行解析：name: / description: / version: 行
    for line in yaml_block.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name:") {
            parsed.name = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(rest) = trimmed.strip_prefix("description:") {
            parsed.description = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(rest) = trimmed.strip_prefix("version:") {
            parsed.version = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    // 提取正文：跳过结束的 `\n---`（4 字节）与紧随其后的单个换行
    let after_frontmatter = &after_start[end + 4..];
    let body = after_frontmatter
        .strip_prefix('\n')
        .or_else(|| after_frontmatter.strip_prefix("\r\n"))
        .unwrap_or(after_frontmatter);
    parsed.body = body.to_string();
    parsed
}

/// `parse_skill_md` 的返回结构
#[derive(Debug, Default, Clone)]
pub struct ParsedSkillMd {
    pub name: String,
    pub description: String,
    pub version: String,
    /// 整个 SKILL.md 内容（含 frontmatter）
    pub preamble: String,
    /// SKILL.md 正文（去除 frontmatter 后的内容）。
    /// 无 frontmatter 时与 `preamble` 一致。
    /// 作为 LLM 系统消息注入时使用此字段，避免 YAML 噪声污染上下文。
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_md_with_frontmatter() {
        let content = "---\nname: weather\ndescription: Get current weather\nversion: 1.0.0\n---\n# Weather\nQuick one-liner:\n```\ncurl wttr.in\n```\n";
        let parsed = parse_skill_md(content);
        assert_eq!(parsed.name, "weather");
        assert_eq!(parsed.description, "Get current weather");
        assert_eq!(parsed.version, "1.0.0");
        assert!(parsed.preamble.contains("# Weather"));
        // body 应去除 frontmatter，仅保留正文
        assert!(!parsed.body.starts_with("---"));
        assert!(parsed.body.starts_with("# Weather"));
        assert!(parsed.body.contains("curl wttr.in"));
    }

    #[test]
    fn parse_skill_md_without_frontmatter() {
        let content = "# Plain skill\nNo frontmatter here.";
        let parsed = parse_skill_md(content);
        assert!(parsed.name.is_empty());
        assert!(parsed.description.is_empty());
        assert_eq!(parsed.preamble, content);
        // 无 frontmatter 时 body 等于 preamble
        assert_eq!(parsed.body, content);
    }

    #[test]
    fn parse_skill_md_body_strips_crlf_newline() {
        // 验证 CRLF 行尾下 body 仍能正确剥离 frontmatter
        let content = "---\r\nname: x\r\n---\r\nbody line";
        let parsed = parse_skill_md(content);
        assert_eq!(parsed.name, "x");
        assert_eq!(parsed.body, "body line");
    }
}
