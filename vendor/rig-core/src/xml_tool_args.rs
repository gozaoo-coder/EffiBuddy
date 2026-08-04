//! EffiSuite vendored patch: XML 形式的 AI 工具参数解析。
//!
//! 让全部 AI 工具在 JSON 之外多支持一种 **XML 输入**，缓解 LLM 生成 JSON
//! 工具参数时的转义问题（代码 / 正则 / 路径里的引号、反斜杠、换行容易写错）。
//!
//! ## 格式
//!
//! 参数用 `<_KEY_>value</_KEY_>` 形式的标签包裹，`_KEY_`（下划线包裹的名字）
//! 是为了与常见的 HTML/XML 标签（如 `<content>`、`<path>`）区分开：
//!
//! ```xml
//! <_PATH_>src/main.rs</_PATH_>
//! <_START_LINE_>3</_START_LINE_>
//! <_TEXT_>替换为这行内容</_TEXT_>
//! ```
//!
//! 规则：
//! - 标签名大小写不敏感：`_PATH_` 与 `_path_` 等价；键名统一转小写，
//!   与工具参数（serde snake_case 字段名）对齐。
//! - 纯文本值自动推断类型：`true`/`false` → 布尔；整数 → 数字；
//!   浮点 → 数字；其余 → 字符串（首尾空白修剪）。
//! - CDATA 内的内容**原样保留**（不修剪、不推断类型、不做实体解码），
//!   适合存放 `123`、`true` 这类需要保持字符串形态的内容。
//! - 嵌套元素 → 嵌套对象；同一层 ≥2 个同名包裹元素 → 数组（见下例）。
//!
//! 示例（edit_file 的 edits 数组）：
//! ```xml
//! <_PATH_>src/main.rs</_PATH_>
//! <_EDITS_>
//!   <_ITEM_>
//!     <_START_LINE_>3</_START_LINE_>
//!     <_TEXT_>第一处替换</_TEXT_>
//!   </_ITEM_>
//!   <_ITEM_>
//!     <_END_LINE_>5</_END_LINE_>
//!     <_TEXT_>第二处替换</_TEXT_>
//!   </_ITEM_>
//! </_EDITS_>
//! ```
//!
//! 等价 JSON：`{"path":"src/main.rs","edits":[{"start_line":3,"text":"第一处替换"},
//! {"end_line":5,"text":"第二处替换"}]}`。
//!
//! ## 与 JSON 的关系
//!
//! `json_utils::parse_tool_arguments` 先按 JSON 解析，失败时才回退到本模块，
//! 因此 JSON 调用完全不受影响；只有模型输出 XML 时才走 XML 路径。

use serde_json::{Map, Value};

/// 把 `<_KEY_>` 风格的 XML 工具参数解析为 JSON 对象。
///
/// 失败（无有效元素 / 结构畸形）返回 `None`，调用方保留原有 JSON 错误。
pub fn parse_xml_tool_arguments(input: &str) -> Option<Value> {
    let mut parser = Parser::new(input);
    let children = parser.parse_children(None).ok()?;
    if children.is_empty() {
        return None;
    }
    container_from_children(&children)
}

/// 一个解析出的元素：小写化后的键 + 内容片段序列。
#[derive(Debug, Clone, PartialEq)]
struct RawChild {
    name: String,
    frags: Vec<Frag>,
    /// 是否自闭合 `<_X_/>`（无开闭标签对，值为 null）
    self_closing: bool,
}

/// 元素内容中的一个片段。
#[derive(Debug, Clone, PartialEq)]
enum Frag {
    /// 普通文本（实体已解码）。
    Text(String),
    /// CDATA 原文（不解析实体、不做类型推断）。
    CData(String),
    /// 嵌套元素。
    Element(RawChild),
}

/// 手写递归下降解析器，零依赖、仅处理我们约定的 `<_NAME_>` 子集。
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

/// CDATA 开标签；用 concat 拼装避免源码出现完整字面量
const CDATA_OPEN: &str = concat!("<!", "[CDATA[");
/// CDATA 闭标签
const CDATA_CLOSE: &str = "]]>";

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.rest().chars().next() {
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    /// 解析一层兄弟元素，直到 EOF（`close_tag == None`）或命中 `close_tag`
    /// （消费之）。容器层对元素之间的杂散文本采取宽容策略：跳过。
    fn parse_children(&mut self, close_tag: Option<&str>) -> Result<Vec<RawChild>, ()> {
        let mut children = Vec::new();
        loop {
            if let Some(ct) = close_tag {
                if self.starts_with(ct) {
                    self.pos += ct.len();
                    return Ok(children);
                }
            }
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                if close_tag.is_some() {
                    return Err(());
                }
                return Ok(children);
            }
            if self.starts_with("<_") {
                children.push(self.parse_element()?);
                continue;
            }
            if self.starts_with("</") {
                // 非预期的闭合标签：结构畸形
                return Err(());
            }
            // 容器层出现杂散文本（如 markdown 代码围栏 / 解释性前言）：
            // 宽容地跳到下一个 `<`，避免把非元素内容当成键值。
            let rest = self.rest();
            if rest.starts_with('<') {
                self.pos += 1;
                continue;
            }
            let next_lt = rest.find('<').unwrap_or(rest.len());
            if next_lt == 0 {
                return Err(());
            }
            self.pos += next_lt;
        }
    }

    /// 解析一个 `<_NAME_>...</_NAME_>` 或 `<_NAME_/>` 元素。
    fn parse_element(&mut self) -> Result<RawChild, ()> {
        // 当前在 "<_"；消费 "<_"
        self.pos += 2;
        let rest = self.rest();
        // 名字是 `<_` 与 `_>`（或 `_/>`）之间的内容，名字里可含下划线
        // （如 `_START_LINE_`），所以扫描第一个 `_>` / `_/>` 分隔符。
        let self_close = rest.find("_/>");
        let normal_close = rest.find("_>");
        let delim = match (self_close, normal_close) {
            (Some(s), Some(n)) => s.min(n),
            (Some(s), None) => s,
            (None, Some(n)) => n,
            (None, None) => return Err(()),
        };
        let name = &rest[..delim];
        if name.is_empty() {
            return Err(());
        }
        let is_self_closing = self_close == Some(delim);
        self.pos += delim;
        if is_self_closing {
            // 消费 "_/>"
            self.pos += 3;
            return Ok(RawChild {
                name: name.to_lowercase(),
                frags: Vec::new(),
                self_closing: true,
            });
        }
        // 消费 "_>"
        self.pos += 2;
        let close_tag = format!("</_{name}_>");
        let frags = self.parse_content(&close_tag)?;
        Ok(RawChild {
            name: name.to_lowercase(),
            frags,
            self_closing: false,
        })
    }

    /// 解析元素内容（文本 / CDATA / 嵌套元素）直到 `close_tag`（消费之）。
    fn parse_content(&mut self, close_tag: &str) -> Result<Vec<Frag>, ()> {
        let mut frags: Vec<Frag> = Vec::new();
        let mut text = String::new();
        // 把累积的普通文本解码实体后压入片段
        macro_rules! flush_text {
            () => {
                if !text.is_empty() {
                    frags.push(Frag::Text(decode_entities(&std::mem::take(&mut text))));
                }
            };
        }
        loop {
            if self.starts_with(close_tag) {
                flush_text!();
                self.pos += close_tag.len();
                return Ok(frags);
            }
            if self.pos >= self.input.len() {
                return Err(());
            }
            if self.starts_with(CDATA_OPEN) {
                flush_text!();
                self.pos += CDATA_OPEN.len();
                let start = self.pos;
                match self.rest().find(CDATA_CLOSE) {
                    Some(idx) => {
                        frags.push(Frag::CData(self.input[start..start + idx].to_string()));
                        self.pos = start + idx + CDATA_CLOSE.len();
                    }
                    None => {
                        // CDATA 未闭合：按"其余全部内容"处理，避免丢失
                        frags.push(Frag::CData(self.input[start..].to_string()));
                        self.pos = self.input.len();
                    }
                }
                continue;
            }
            if self.starts_with("<_") {
                // 尝试作为嵌套元素解析；失败则把 `<` 当作文本字符（宽松）
                let saved = self.pos;
                match self.parse_element() {
                    Ok(child) => {
                        flush_text!();
                        frags.push(Frag::Element(child));
                    }
                    Err(()) => {
                        self.pos = saved;
                        text.push('<');
                        self.pos += 1;
                    }
                }
                continue;
            }
            if self.starts_with("</") {
                return Err(());
            }
            // 逐字符累积文本（含裸 `<`/`>`，这正是 XML 输入免转义的价值）
            let ch = self.rest().chars().next().ok_or(())?;
            text.push(ch);
            self.pos += ch.len_utf8();
        }
    }
}

/// 把一列兄弟元素合并为 JSON 对象；同名重复 → 数组。
fn container_from_children(children: &[RawChild]) -> Option<Value> {
    let mut map: Map<String, Value> = Map::new();
    for child in children {
        let value = child_value(child);
        match map.get_mut(&child.name) {
            None => {
                map.insert(child.name.clone(), value);
            }
            Some(existing) => {
                let prev = existing.take();
                *existing = Value::Array(vec![prev, value]);
            }
        }
    }
    if map.is_empty() {
        None
    } else {
        Some(Value::Object(map))
    }
}

/// 计算单个元素的值。
fn child_value(child: &RawChild) -> Value {
    // 自闭合 `<_X_/>` → null
    if child.self_closing {
        return Value::Null;
    }
    // 含嵌套元素 → 递归构造对象/数组（忽略夹杂的文本）
    if child.frags.iter().any(|f| matches!(f, Frag::Element(_))) {
        let elems: Vec<RawChild> = child
            .frags
            .iter()
            .filter_map(|f| match f {
                Frag::Element(e) => Some(e.clone()),
                _ => None,
            })
            .collect();
        // 数组约定：同一层同名包裹元素（如 `<ITEM>` 重复）→ 直接展开为数组，
        // 包裹名只是占位，不进入键。
        // - ≥2 个同名：任意名字都视为数组
        // - 恰好 1 个且名字是通用占位（item/entry/row/value/element/纯数字）：
        //   也视为单元素数组，避免“edits 只改一处”写成单个 `<ITEM>` 时丢字段
        if elems.len() >= 2 && elems.iter().all(|e| e.name == elems[0].name) {
            return Value::Array(elems.iter().map(child_value).collect());
        }
        if elems.len() == 1 && is_array_placeholder(&elems[0].name) {
            return Value::Array(vec![child_value(&elems[0])]);
        }
        return container_from_children(&elems).unwrap_or(Value::Null);
    }
    // 纯文本 / CDATA：拼接
    let mut has_cdata = false;
    let mut s = String::new();
    for f in &child.frags {
        match f {
            Frag::Text(t) => s.push_str(t),
            Frag::CData(c) => {
                has_cdata = true;
                s.push_str(c);
            }
            Frag::Element(_) => {}
        }
    }
    if has_cdata {
        // CDATA 出现即视为显式字符串：不修剪、不推断类型
        Value::String(s)
    } else {
        coerce_scalar(&s)
    }
}

/// 通用数组元素占位名：单个此类名字的元素也展开为数组（而非对象）。
fn is_array_placeholder(name: &str) -> bool {
    matches!(
        name,
        "item" | "entry" | "row" | "value" | "element" | "elem" | "obj"
    ) || !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())
}


/// 纯文本 → 类型推断：bool → i64 → f64 → 字符串。
fn coerce_scalar(text: &str) -> Value {
    let t = text.trim();
    if t.is_empty() {
        return Value::String(String::new());
    }
    if t.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(i) = t.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = t.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return Value::Number(n);
    }
    Value::String(t.to_string())
}

/// 解码标准 XML 实体（`&amp;` 最后解，保证 `&amp;lt;` → `&lt;`）。
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(input: &str) -> Value {
        parse_xml_tool_arguments(input).expect("xml should parse")
    }

    #[test]
    fn flat_key_values() {
        let input = r#"
            <_PATH_>src/main.rs</_PATH_>
            <_START_LINE_>3</_START_LINE_>
            <_TEXT_>替换内容</_TEXT_>
        "#;
        assert_eq!(
            parse(input),
            json!({
                "path": "src/main.rs",
                "start_line": 3,
                "text": "替换内容"
            })
        );
    }

    #[test]
    fn uppercase_and_lowercase_tags_are_equivalent() {
        assert_eq!(
            parse("<_PATH_>a</_PATH_>"),
            parse("<_path_>a</_path_>")
        );
        assert_eq!(parse("<_PATH_>a</_PATH_>"), json!({"path": "a"}));
    }

    #[test]
    fn scalar_type_inference() {
        assert_eq!(parse("<_A_>true</_A_>"), json!({"a": true}));
        assert_eq!(parse("<_A_>FALSE</_A_>"), json!({"a": false}));
        assert_eq!(parse("<_A_>42</_A_>"), json!({"a": 42}));
        assert_eq!(parse("<_A_>-7</_A_>"), json!({"a": -7}));
        assert_eq!(parse("<_A_>3.14</_A_>"), json!({"a": 3.14}));
        assert_eq!(parse("<_A_>hello</_A_>"), json!({"a": "hello"}));
    }

    #[test]
    fn cdata_forces_string_and_preserves_verbatim() {
        let input =
            concat!("<_CONTENT_>", "<![", "CDATA[123 \"quoted\" <html> & more]]></_CONTENT_>");
        assert_eq!(
            parse(input),
            json!({"content": r#"123 "quoted" <html> & more"#})
        );
        // CDATA 内的纯数字也保持字符串
        let num = concat!("<_CONTENT_>", "<![", "CDATA[123]]></_CONTENT_>");
        assert_eq!(parse(num), json!({"content": "123"}));
    }

    #[test]
    fn entities_are_decoded_in_plain_text() {
        assert_eq!(
            parse("<_TEXT_>a &amp; b &lt;c&gt;</_TEXT_>"),
            json!({"text": "a & b <c>"})
        );
    }

    #[test]
    fn raw_angle_brackets_in_plain_text() {
        assert_eq!(
            parse("<_CONTENT_>fn f() { if a < b && c > d }</_CONTENT_>"),
            json!({"content": "fn f() { if a < b && c > d }"})
        );
    }

    #[test]
    fn nested_object() {
        let input = r#"
            <_PATH_>a.txt</_PATH_>
            <_EDIT_>
                <_START_LINE_>1</_START_LINE_>
                <_TEXT_>line</_TEXT_>
            </_EDIT_>
        "#;
        assert_eq!(
            parse(input),
            json!({"path": "a.txt", "edit": {"start_line": 1, "text": "line"}})
        );
    }

    #[test]
    fn repeated_siblings_become_array() {
        let input = r#"
            <_KEYWORDS_>rust</_KEYWORDS_>
            <_KEYWORDS_>async</_KEYWORDS_>
        "#;
        assert_eq!(parse(input), json!({"keywords": ["rust", "async"]}));
    }

    #[test]
    fn array_of_objects_with_item_wrapper() {
        let input = r#"
            <_PATH_>src/main.rs</_PATH_>
            <_EDITS_>
                <_ITEM_>
                    <_START_LINE_>3</_START_LINE_>
                    <_TEXT_>第一处</_TEXT_>
                </_ITEM_>
                <_ITEM_>
                    <_END_LINE_>5</_END_LINE_>
                    <_TEXT_>第二处</_TEXT_>
                </_ITEM_>
            </_EDITS_>
        "#;
        assert_eq!(
            parse(input),
            json!({
                "path": "src/main.rs",
                "edits": [
                    {"start_line": 3, "text": "第一处"},
                    {"end_line": 5, "text": "第二处"}
                ]
            })
        );
    }

    #[test]
    fn single_item_wrapper_is_not_unwrapped() {
        // 只有一个包裹元素时保持对象形态（数组至少需要 2 个同名元素）
        let input = r#"
            <_EDIT_>
                <_START_LINE_>3</_START_LINE_>
                <_TEXT_>仅一处</_TEXT_>
            </_EDIT_>
        "#;
        assert_eq!(
            parse(input),
            json!({"edit": {"start_line": 3, "text": "仅一处"}})
        );
    }

    #[test]
    fn self_closing_tag_is_null() {
        assert_eq!(parse("<_OPTIONAL_/>"), json!({"optional": null}));
    }

    #[test]
    fn empty_element_is_empty_string() {
        assert_eq!(parse("<_A_></_A_>"), json!({"a": ""}));
    }

    #[test]
    fn markdown_fence_and_prose_are_tolerated() {
        let input = r#"工具参数如下：
```xml
<_PATH_>a.txt</_PATH_>
```
"#;
        assert_eq!(parse(input), json!({"path": "a.txt"}));
    }

    #[test]
    fn empty_and_garbage_return_none() {
        assert!(parse_xml_tool_arguments("").is_none());
        assert!(parse_xml_tool_arguments("   \n  ").is_none());
        assert!(parse_xml_tool_arguments("plain text only").is_none());
        assert!(parse_xml_tool_arguments("<>not ours</>").is_none());
    }

    #[test]
    fn unterminated_element_returns_none() {
        assert!(parse_xml_tool_arguments("<_PATH_>abc").is_none());
        assert!(parse_xml_tool_arguments("<_PATH_>").is_none());
    }

    #[test]
    fn names_with_underscores_work() {
        let input = r#"
            <_START_LINE_>2</_START_LINE_>
            <_DIFF_CONTEXT_>3</_DIFF_CONTEXT_>
            <_CASE_SENSITIVE_>true</_CASE_SENSITIVE_>
        "#;
        assert_eq!(
            parse(input),
            json!({
                "start_line": 2,
                "diff_context": 3,
                "case_sensitive": true
            })
        );
    }

    #[test]
    fn whitespace_in_values_is_trimmed() {
        assert_eq!(parse("<_A_>  hello  </_A_>"), json!({"a": "hello"}));
    }

    #[test]
    fn single_placeholder_item_is_unwrapped_to_array() {
        // 通用占位名 `ITEM` 即使只有一个元素也展开为数组（edits 只改一处）
        let input = r#"
            <_EDITS_>
                <_ITEM_>
                    <_START_LINE_>3</_START_LINE_>
                    <_TEXT_>仅一处</_TEXT_>
                </_ITEM_>
            </_EDITS_>
        "#;
        assert_eq!(
            parse(input),
            json!({"edits": [{"start_line": 3, "text": "仅一处"}]})
        );
    }

    #[test]
    fn single_line_flat() {
        assert_eq!(
            parse("<_PATH_>a.txt</_PATH_><_APPEND_>true</_APPEND_>"),
            json!({"path": "a.txt", "append": true})
        );
    }
}
