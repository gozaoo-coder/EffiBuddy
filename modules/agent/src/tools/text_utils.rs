//! 文本工具共享辅助函数
//!
//! - [`extract_content`]：从 LLM 传入的 content 字段提取实际文本（XML/CDATA 包裹剥离），
//!   供 write_file / edit_file 共用，避免 LLM 写代码时纠结 JSON 转义
//! - [`numbered_lines`] / [`format_numbered_line`]：给文本逐行加行号
//!   （1-based、右对齐、两空格分隔），供 read_file / search_file / edit_file 共用，
//!   保证"行号 ↔ 行内容"格式全局一致，让 LLM 能按 search/read 返回的行号
//!   精确调用 edit_file 编辑目标行

/// 内容提取的分隔符
const CONTENT_OPEN: &str = "<content>";
const CONTENT_CLOSE: &str = "</content>";
const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";

/// 从 LLM 传入的 content 字段中提取实际文本内容
///
/// 识别优先级：
/// 1. **CDATA 包裹**：`<content><![CDATA[...]]></content>` 或裸 `<![CDATA[...]]>`
///    - CDATA 内部所有字符原样保留，唯一不能出现的是字面量 `]]>`
/// 2. **XML 标签包裹**：`<content>...</content>`
///    - 标签之间的文本原样提取，**不做 XML 实体解码**，`<` `>` `&` 无需转义
/// 3. **裸文本**：不含任何上述标签时直接返回原文（向后兼容）
///
/// 容错：找不到闭合标签时退化为"去掉开标签后的全部内容"，避免丢失。
pub(crate) fn extract_content(raw: &str) -> String {
    // 优先识别 CDATA：`<![CDATA[...]]>`
    if let Some(cdata_start) = raw.find(CDATA_OPEN) {
        let inner_start = cdata_start + CDATA_OPEN.len();
        if let Some(cdata_end) = raw[inner_start..].find(CDATA_CLOSE) {
            let inner_end = inner_start + cdata_end;
            return raw[inner_start..inner_end].to_string();
        }
        // CDATA 未闭合：按"裸 CDATA 后所有内容"处理，避免丢失
        tracing::warn!("extract_content: CDATA 未闭合，按裸内容处理");
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
        tracing::warn!("extract_content: <content> 未闭合，按裸内容处理");
        return raw[inner_start..].to_string();
    }

    // 裸文本：直接返回（调用方已 JSON 解码过 \n \" 等）
    raw.to_string()
}

/// 计算行号右对齐宽度：按最大行号（总行数）的十进制位数，至少 1 位
#[inline]
pub(crate) fn line_number_width(total_lines: usize) -> usize {
    total_lines.max(1).to_string().len()
}

/// 格式化一行：`行号（右对齐） + 两个空格 + 行内容`
///
/// 示例（width=4）：
/// ```text
///   12  fn main() {
///  123  let x = 1;
/// ```
#[inline]
pub(crate) fn format_numbered_line(line_no: usize, width: usize, content: &str) -> String {
    format!("{line_no:>width$}  {content}")
}

/// 把全文转为带行号文本（1-based 行号，右对齐，两空格分隔）
///
/// 用 `str::lines()` 切分（自动处理 \r\n，且不产生末尾多余空行）。
/// 行号宽度按全文总行数计算，保证同文件内对齐一致。
///
/// 注意：read_file / search_file 目前按行流式格式化（支持行范围/命中过滤），
/// 此函数保留给需要整篇带行号输出的场景（如测试断言）。
#[cfg(test)]
pub(crate) fn numbered_lines(content: &str) -> String {
    let total = content.lines().count();
    let width = line_number_width(total);
    let mut out = String::with_capacity(content.len() + total * (width + 2));
    for (i, line) in content.lines().enumerate() {
        out.push_str(&format_numbered_line(i + 1, width, line));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ extract_content（自 write_file 迁移） ============

    #[test]
    fn extract_xml_wrapped_content() {
        let raw = "<content>hello world</content>";
        assert_eq!(extract_content(raw), "hello world");
    }

    #[test]
    fn extract_xml_wrapped_preserves_special_chars() {
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
        let raw = "<content><![CDATA[<content>嵌套标签</content>]]></content>";
        assert_eq!(extract_content(raw), "<content>嵌套标签</content>");
    }

    #[test]
    fn extract_bare_cdata_without_content_tag() {
        let raw = "<![CDATA[裸 CDATA 内容]]>";
        assert_eq!(extract_content(raw), "裸 CDATA 内容");
    }

    #[test]
    fn extract_unclosed_content_fallback() {
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
        let raw = "plain text content";
        assert_eq!(extract_content(raw), "plain text content");
    }

    #[test]
    fn extract_empty_content() {
        assert_eq!(extract_content("<content></content>"), "");
        assert_eq!(extract_content("<content><![CDATA[]]></content>"), "");
    }

    #[test]
    fn extract_xml_with_leading_text() {
        let raw = "这是文件内容：\n<content>实际内容</content>";
        assert_eq!(extract_content(raw), "实际内容");
    }

    #[test]
    fn extract_cdata_takes_priority_over_content_tag() {
        let raw = "<content><![CDATA[CDATA 内容]]></content>";
        assert_eq!(extract_content(raw), "CDATA 内容");
    }

    // ============ 行号格式化 ============

    #[test]
    fn numbered_lines_basic() {
        let out = numbered_lines("fn main() {\n    println!(\"hi\");\n}");
        assert_eq!(
            out,
            "1  fn main() {\n2      println!(\"hi\");\n3  }\n"
        );
    }

    #[test]
    fn numbered_lines_aligns_width_by_total() {
        // 10 行 → 宽度 2
        let content = (1..=10).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        let out = numbered_lines(&content);
        assert!(out.starts_with(" 1  line1\n 2  line2\n"));
        assert!(out.contains("\n10  line10\n"));
    }

    #[test]
    fn numbered_lines_handles_crlf_and_trailing_newline() {
        // \r\n 自动归一为 \n；末尾换行不产生多余空行
        let out = numbered_lines("a\r\nb\r\n");
        assert_eq!(out, "1  a\n2  b\n");
    }

    #[test]
    fn numbered_lines_empty() {
        assert_eq!(numbered_lines(""), "");
    }
}
