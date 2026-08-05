//! XML 工具参数输入 端到端测试（EffiSuite vendored patch）
//!
//! 验证模型以 XML 形式（`<_KEY_>` 标签）输出工具参数时，整条链路可用：
//! OpenAI 兼容 provider 的 `Function`（`deserialize_maybe_stringified` → JSON 失败
//! 回退 XML）→ 产出 JSON 对象 → serde 反序列化为本工具的强类型 `EditFileArgs`。

use rig_core::providers::openai::completion::Function;
use serde_json::json;

use super::types::EditFileArgs;

/// 模型输出 `arguments` 为 XML 字符串时，provider 解析出的对象应正确，
/// 且能直接反序列化为 `EditFileArgs`（含 edits 数组 + CDATA 文本）。
#[test]
fn xml_arguments_round_trip_to_typed_args() {
    // 模拟 provider 响应里的 function.arguments 字段（字符串形式的 XML）
    let wire = r#"{"name":"edit_file","arguments":"<_PATH_>src/main.rs</_PATH_><_EDITS_><_ITEM_><_START_LINE_>3</_START_LINE_><_TEXT_><![CDATA[fn main() { let s = \"a < b\"; }]]></_TEXT_></_ITEM_><_ITEM_><_END_LINE_>9</_END_LINE_><_TEXT_>追加的代码</_TEXT_></_ITEM_></_EDITS_>"}"#;

    let function: Function = serde_json::from_str(wire).expect("arguments 应为可解析的 XML");
    assert_eq!(function.name, "edit_file");
    assert_eq!(
        function.arguments,
        json!({
            "path": "src/main.rs",
            "edits": [
                {"start_line": 3, "text": "fn main() { let s = \"a < b\"; }"},
                {"end_line": 9, "text": "追加的代码"}
            ]
        })
    );

    // 反序列化为强类型参数
    let args: EditFileArgs =
        serde_json::from_value(function.arguments).expect("XML 解析结果应能反序列化为 EditFileArgs");
    assert_eq!(args.path, "src/main.rs");
    assert_eq!(args.edits.len(), 2);
    assert_eq!(args.edits[0].start_line, Some(3));
    assert_eq!(args.edits[0].text, "fn main() { let s = \"a < b\"; }");
    assert_eq!(args.edits[1].end_line, Some(9));
    assert_eq!(args.edits[1].text, "追加的代码");
}
    /// 顶层平坦标量：布尔 / 数字 / 字符串自动识别。
    #[test]
    fn xml_flat_scalars_deserialize() {
        let wire = r#"{"name":"edit_file","arguments":"<_PATH_>a.rs</_PATH_><_DRY_RUN_>true</_DRY_RUN_><_DIFF_CONTEXT_>2</_DIFF_CONTEXT_><_EDITS_><_ITEM_><_INSERT_BEFORE_>1</_INSERT_BEFORE_><_TEXT_>x</_TEXT_></_ITEM_></_EDITS_>"}"#;
        let function: Function = serde_json::from_str(wire).unwrap();
        let args: EditFileArgs = serde_json::from_value(function.arguments).unwrap();
        assert_eq!(args.path, "a.rs");
        assert_eq!(args.dry_run, Some(true));
        assert_eq!(args.diff_context, Some(2));
        assert_eq!(args.edits[0].insert_before, Some(1));
    }

    /// 新格式 `<!_KEY_>`（推荐）：provider 链路同样可用。
    #[test]
    fn xml_new_style_round_trip_to_typed_args() {
        let wire = r#"{"name":"edit_file","arguments":"<!_PATH_>src/main.rs</!_PATH_><!_EDITS_><!_ITEM_><!_START_LINE_>3</!_START_LINE_><!_TEXT_>fn main() {}</!_TEXT_></!_ITEM_></!_EDITS_>"}"#;
        let function: Function = serde_json::from_str(wire).unwrap();
        assert_eq!(function.name, "edit_file");
        assert_eq!(
            function.arguments,
            json!({
                "path": "src/main.rs",
                "edits": [{"start_line": 3, "text": "fn main() {}"}]
            })
        );
        let args: EditFileArgs = serde_json::from_value(function.arguments).unwrap();
        assert_eq!(args.path, "src/main.rs");
        assert_eq!(args.edits[0].start_line, Some(3));
    }
