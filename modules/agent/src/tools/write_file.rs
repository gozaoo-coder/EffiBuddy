//! write_file 工具：让 LLM 写入本地文件
//!
//! 与 [`ReadFileTool`] 对称，提供"写"能力。核心难点是 **内容转义**：
//! LLM 通过 JSON 工具参数传内容时，JSON 字符串里的 `"`、`\`、换行
//! 都需要转义；当内容本身是代码或 XML/HTML 时，LLM 容易写错转义
//! 导致写入内容被破坏。
//!
//! 本工具的解决方案：**用 XML 包裹 + 原文提取**。
//! LLM 把文件内容放在 `<content>...</content>` 标签之间，工具从标签
//! 之间提取**原始文本**（不做 XML 实体解码），让 `<`/`>`/`&` 等字符
//! 无需转义即可写入。同时也支持 CDATA 写法 `<![CDATA[...]]>`。
//!
//! ## 为什么不用 JSON 字符串直接传？
//! - 代码中的 `"` 需要 `\"` 转义，LLM 经常漏写或多写
//! - 正则表达式、Windows 路径中的 `\` 需要 `\\`，LLM 容易出错
//! - JSON 字符串不允许多行字面量，所有换行必须是 `\n`
//!
//! ## 为什么不用 base64？
//! - LLM 不会算 base64，只能"看起来像"base64，几乎必然写错
//! - 不可读，调试困难
//!
//! ## XML 包裹方案的安全性
//! - `<content>` 与 `</content>` 作为分隔符，提取两者之间的文本
//! - 内容中的 `<` `>` `&` 原样保留，不做实体解码
//! - 若内容本身包含 `</content>`，LLM 应改用 CDATA：`<![CDATA[...]]>`
//!   工具优先识别 CDATA，其次识别普通标签包裹
//!
//! 工作区支持：构造时传入 `cwd: Option<PathBuf>`，相对路径会 join 到 cwd。
//! 信任本地 agent 环境，路径不做沙箱限制（与 read_file 一致）。

use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::fs;

use super::resolve_path;

/// 单次写入最大字节数（1 MiB），防止 LLM 误写入超大内容撑爆磁盘
const MAX_WRITE_BYTES: usize = 1024 * 1024;

/// 工具参数
///
/// 字段按大小降序：String（24B）> Option<bool>（1B）。
///
/// `content` 字段接受两种格式（工具自动识别）：
/// 1. XML 包裹：`<content>文件原始内容</content>`（推荐，无需转义）
/// 2. CDATA 包裹：`<content><![CDATA[文件原始内容]]></content>`
/// 3. 裸文本：直接传文件内容（向后兼容，但需 JSON 转义）
#[derive(Deserialize)]
pub struct WriteFileArgs {
    /// 要写入的文件路径（绝对或相对工作区）
    pub path: String,
    /// 文件内容。推荐用 `<content>...</content>` 包裹以避免转义
    pub content: String,
    /// 是否追加写入而非覆盖，默认 false（覆盖）
    #[serde(default)]
    pub append: Option<bool>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("write_file error: {0}")]
pub struct WriteFileError(String);

/// 文件写入工具
///
/// `cwd` 为可选工作区：设置后相对路径以此为基准，未设置则依赖进程 cwd。
pub struct WriteFileTool {
    cwd: Option<PathBuf>,
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self { cwd: None }
    }

    /// 指定工作区目录，相对路径将 join 到此目录
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self { cwd: Some(cwd) }
    }
}

impl Default for WriteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

/// 内容提取的分隔符
const CONTENT_OPEN: &str = "<content>";
const CONTENT_CLOSE: &str = "</content>";
const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";

/// 从 LLM 传入的 content 字段中提取实际文件内容
///
/// 识别优先级：
/// 1. **CDATA 包裹**：`<content><![CDATA[...]]></content>` 或裸 `<![CDATA[...]]>`
///    - CDATA 内部的所有字符原样保留，包括 `<` `>` `&`
///    - 唯一不能出现的是字面量 `]]>`
/// 2. **XML 标签包裹**：`<content>...</content>`
///    - 标签之间的文本原样提取，**不做 XML 实体解码**
///    - 内容中的 `<` `>` `&` 无需转义
/// 3. **裸文本**：不含任何上述标签时，直接当作文件内容（向后兼容）
///
/// 设计权衡：
/// - 不引入 xml crate：纯字符串扫描足够，避免依赖膨胀
/// - 不做实体解码：让 LLM 写代码时无需关心 `&lt;` `&amp;` 等转义
/// - 容错：找不到闭合标签时退化为"去掉开标签后的全部内容"，避免写入失败
fn extract_content(raw: &str) -> String {
    // 优先识别 CDATA：`<![CDATA[...]]>`
    // 允许 CDATA 被 <content> 包裹，也允许裸 CDATA
    if let Some(cdata_start) = raw.find(CDATA_OPEN) {
        let inner_start = cdata_start + CDATA_OPEN.len();
        if let Some(cdata_end) = raw[inner_start..].find(CDATA_CLOSE) {
            let inner_end = inner_start + cdata_end;
            return raw[inner_start..inner_end].to_string();
        }
        // CDATA 未闭合：按"裸 CDATA 后所有内容"处理，避免丢失
        tracing::warn!("write_file: CDATA 未闭合，按裸内容处理");
        return raw[inner_start..].to_string();
    }

    // 其次识别 <content>...</content>
    if let Some(open_start) = raw.find(CONTENT_OPEN) {
        let inner_start = open_start + CONTENT_OPEN.len();
        if let Some(close_pos) = raw[inner_start..].find(CONTENT_CLOSE) {
            let inner_end = inner_start + close_pos;
            return raw[inner_start..inner_end].to_string();
        }
        // <content> 未闭合：按"开标签后的全部内容"处理
        tracing::warn!("write_file: <content> 未闭合，按裸内容处理");
        return raw[inner_start..].to_string();
    }

    // 裸文本：直接返回（调用方已 JSON 解码过 \n \" 等）
    raw.to_string()
}

impl Tool for WriteFileTool {
    const NAME: &'static str = "write_file";

    type Error = WriteFileError;
    type Args = WriteFileArgs;
    type Output = String;

    fn description(&self) -> String {
        let cwd_hint = self
            .cwd
            .as_ref()
            .map(|p| format!("当前工作区：{}（相对路径以此为准）", p.display()))
            .unwrap_or_else(|| "未设置工作区，相对路径依赖进程工作目录".to_string());
        format!(
            "写入本地文件。路径不做沙箱限制（信任本地 agent 环境）。\
             默认覆盖写入，设置 append=true 可追加。单次最多写入 1 MiB。\n\n\
             **内容格式（重要，避免转义问题）**：\n\
             推荐把文件内容用 XML 标签包裹，标签内的字符无需任何转义：\n\
             <content>文件原始内容，可含 < > & \" \\ 等任意字符</content>\n\n\
             若内容本身包含字面量 `</content>`，改用 CDATA 包裹：\n\
             <content><![CDATA[任意内容，包括 </content> 字面量]]></content>\n\n\
             不推荐直接传裸文本：需要 JSON 转义（双引号、反斜杠、换行），容易写错。\n\n{cwd_hint}"
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件路径（绝对或相对工作区）"
                },
                "content": {
                    "type": "string",
                    "description": "文件内容。推荐用 <content>...</content> 包裹以避免转义；含 </content> 字面量时用 <content><![CDATA[...]]></content>"
                },
                "append": {
                    "type": "boolean",
                    "description": "是否追加写入而非覆盖，默认 false",
                    "default": false
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let append = args.append.unwrap_or(false);
        let resolved = resolve_path(&args.path, self.cwd.as_deref());

        // 提取实际文件内容（XML/CDATA 包裹剥离）
        let file_content = extract_content(&args.content);

        // 大小限制
        if file_content.len() > MAX_WRITE_BYTES {
            return Err(WriteFileError(format!(
                "内容过大（{} 字节），超过单次写入上限 {} 字节",
                file_content.len(),
                MAX_WRITE_BYTES
            )));
        }

        // 确保父目录存在（append=false 且文件存在时不需要，但覆盖新文件时需要）
        if let Some(parent) = resolved.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| WriteFileError(format!("创建父目录失败 [{}]: {e}", parent.display())))?;
            }
        }

        let write_result = if append {
            // 追加模式：用 OpenOptions 以 append + create 方式打开
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&resolved)
                .await
                .map_err(|e| WriteFileError(format!("打开文件失败 [{}]: {e}", resolved.display())))?;
            file.write_all(file_content.as_bytes())
                .await
                .map_err(|e| WriteFileError(format!("追加写入失败 [{}]: {e}", resolved.display())))?;
            file.flush()
                .await
                .map_err(|e| WriteFileError(format!("flush 失败 [{}]: {e}", resolved.display())))?;
            file_content.len()
        } else {
            fs::write(&resolved, file_content.as_bytes())
                .await
                .map_err(|e| WriteFileError(format!("写入文件失败 [{}]: {e}", resolved.display())))?;
            file_content.len()
        };

        Ok(format!(
            "已{}写入 {} ({} 字节)",
            if append { "追加" } else { "覆盖" },
            resolved.display(),
            write_result
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ extract_content ============

    #[test]
    fn extract_xml_wrapped_content() {
        let raw = "<content>hello world</content>";
        assert_eq!(extract_content(raw), "hello world");
    }

    #[test]
    fn extract_xml_wrapped_preserves_special_chars() {
        // 内容含 < > & " \ 等无需转义
        let raw = r#"<content>const s = "a < b && c > d"; regex = /\d+/</content>"#;
        assert_eq!(extract_content(raw), r#"const s = "a < b && c > d"; regex = /\d+/"#);
    }

    #[test]
    fn extract_xml_wrapped_preserves_newlines() {
        let raw = "<content>line1\nline2\nline3</content>";
        assert_eq!(extract_content(raw), "line1\nline2\nline3");
    }

    #[test]
    fn extract_cdata_wrapped() {
        let raw = "<content><![CDATA[hello <world> & friends]]></content>";
        assert_eq!(extract_content(raw), "hello <world> & friends");
    }

    #[test]
    fn extract_cdata_with_content_literal() {
        // 内容含 </content> 字面量，必须用 CDATA
        let raw = "<content><![CDATA[<content>嵌套标签</content>]]></content>";
        assert_eq!(extract_content(raw), "<content>嵌套标签</content>");
    }

    #[test]
    fn extract_cdata_with_cdata_close_escape() {
        // CDATA 内不能含字面量 ]]>，这是 XML 规范限制
        // 若出现，提取到第一个 ]]> 为止
        let raw = "<content><![CDATA[before ]]> after]]></content>";
        assert_eq!(extract_content(raw), "before ");
    }

    #[test]
    fn extract_bare_cdata_without_content_tag() {
        // 也支持裸 CDATA（无 <content> 包裹）
        let raw = "<![CDATA[裸 CDATA 内容]]>";
        assert_eq!(extract_content(raw), "裸 CDATA 内容");
    }

    #[test]
    fn extract_unclosed_content_fallback() {
        // <content> 未闭合：取开标签后的全部内容
        let raw = "<content>未闭合的内容";
        assert_eq!(extract_content(raw), "未闭合的内容");
    }

    #[test]
    fn extract_unclosed_cdata_fallback() {
        let raw = "<content><![CDATA[未闭合";
        assert_eq!(extract_content(raw), "未闭合");
    }

    #[test]
    fn extract_bare_text_passthrough() {
        // 无任何标签包裹，直接返回原文
        let raw = "plain text content";
        assert_eq!(extract_content(raw), "plain text content");
    }

    #[test]
    fn extract_bare_text_with_xml_like_fragments() {
        // 内容含 `<` 但不是 <content>/<![CDATA[ 开头，按裸文本处理
        let raw = "a < b && c > d";
        assert_eq!(extract_content(raw), "a < b && c > d");
    }

    #[test]
    fn extract_empty_content() {
        assert_eq!(extract_content("<content></content>"), "");
        assert_eq!(extract_content("<content><![CDATA[]]></content>"), "");
    }

    #[test]
    fn extract_xml_with_leading_text() {
        // LLM 可能在 <content> 前加说明文字，应忽略
        let raw = "这是文件内容：\n<content>实际内容</content>";
        assert_eq!(extract_content(raw), "实际内容");
    }

    #[test]
    fn extract_cdata_takes_priority_over_content_tag() {
        // 同时有 <content> 和 <![CDATA[ 时，CDATA 优先
        let raw = "<content><![CDATA[CDATA 内容]]></content>";
        assert_eq!(extract_content(raw), "CDATA 内容");
    }

    #[test]
    fn extract_html_file_content() {
        // 模拟 LLM 写 HTML 文件
        let html = r#"<content><!DOCTYPE html>
<html>
<head><title>Test</title></head>
<body>
  <div class="a">hello & bye</div>
  <script>if (a < b) { alert("hi"); }</script>
</body>
</html></content>"#;
        let extracted = extract_content(html);
        assert!(extracted.starts_with("<!DOCTYPE html>"));
        assert!(extracted.contains(r#"class="a""#));
        assert!(extracted.contains("hello & bye"));
        assert!(extracted.contains("a < b"));
        assert!(extracted.contains("</html>"));
    }

    #[test]
    fn extract_rust_source_content() {
        // 模拟 LLM 写 Rust 源码
        let rust = r#"<content>fn main() {
    let s = String::from("hello");
    let arr: Vec<i32> = vec![1, 2, 3];
    if !arr.is_empty() && arr.len() > 0 {
        println!("{}", s);
    }
}</content>"#;
        let extracted = extract_content(rust);
        assert!(extracted.contains(r#"String::from("hello")"#));
        assert!(extracted.contains("Vec<i32>"));
        assert!(extracted.contains("arr.len() > 0"));
        assert!(extracted.contains("}"));
    }

    // ============ 集成测试（文件 IO） ============

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let dir = std::env::temp_dir().join(format!("effisuite-write-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = WriteFileTool::with_cwd(dir.clone());
        let path = "test.txt";

        // 写入
        let result = tool
            .call(WriteFileArgs {
                path: path.to_string(),
                content: "<content>hello & world <test> \"quoted\"</content>".to_string(),
                append: None,
            })
            .await
            .unwrap();
        assert!(result.contains("覆盖写入"));
        assert!(result.contains("test.txt"));

        // 读取验证
        let written = tokio::fs::read_to_string(dir.join(path))
            .await
            .unwrap();
        assert_eq!(written, r#"hello & world <test> "quoted""#);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_append_mode() {
        let dir = std::env::temp_dir().join(format!("effisuite-write-append-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = WriteFileTool::with_cwd(dir.clone());

        // 首次写入
        tool.call(WriteFileArgs {
            path: "log.txt".to_string(),
            content: "<content>line1\n</content>".to_string(),
            append: None,
        })
        .await
        .unwrap();

        // 追加写入
        let result = tool
            .call(WriteFileArgs {
                path: "log.txt".to_string(),
                content: "<content>line2\n</content>".to_string(),
                append: Some(true),
            })
            .await
            .unwrap();
        assert!(result.contains("追加写入"));

        let written = tokio::fs::read_to_string(dir.join("log.txt"))
            .await
            .unwrap();
        assert_eq!(written, "line1\nline2\n");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_creates_parent_dirs() {
        let dir = std::env::temp_dir().join(format!("effisuite-write-mkdir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = WriteFileTool::with_cwd(dir.clone());

        tool.call(WriteFileArgs {
            path: "nested/deep/file.txt".to_string(),
            content: "<content>nested</content>".to_string(),
            append: None,
        })
        .await
        .unwrap();

        let written = tokio::fs::read_to_string(dir.join("nested/deep/file.txt"))
            .await
            .unwrap();
        assert_eq!(written, "nested");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_rejects_oversized_content() {
        let dir = std::env::temp_dir().join(format!("effisuite-write-big-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = WriteFileTool::with_cwd(dir.clone());

        let big = "x".repeat(MAX_WRITE_BYTES + 1);
        let result = tool
            .call(WriteFileArgs {
                path: "big.txt".to_string(),
                content: format!("<content>{big}</content>"),
                append: None,
            })
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("内容过大"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_bare_text_without_xml_wrapper() {
        // 向后兼容：无 <content> 包裹的裸文本也能写入
        let dir = std::env::temp_dir().join(format!("effisuite-write-bare-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = WriteFileTool::with_cwd(dir.clone());

        tool.call(WriteFileArgs {
            path: "bare.txt".to_string(),
            content: "plain text without xml wrapper".to_string(),
            append: None,
        })
        .await
        .unwrap();

        let written = tokio::fs::read_to_string(dir.join("bare.txt"))
            .await
            .unwrap();
        assert_eq!(written, "plain text without xml wrapper");

        std::fs::remove_dir_all(&dir).ok();
    }
}
