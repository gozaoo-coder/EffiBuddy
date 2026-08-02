//! edit_file 报告类测试：迷你 diff / 上下文 / no-op / 空白差异 / 重复插入 / dry_run。

use super::tests_common::{read, setup_file, tmp_dir};
use super::*;
use rig_core::tool::Tool;

#[tokio::test]
async fn report_contains_mini_diff() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "a\nb\nc\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: None,
                insert_before: None,
                text: "B\nB2".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    // 迷你 diff：- 原行（原行号 2） / + 新行（新行号 2-3）
    assert!(r.contains("- 2  b"), "out: {r}");
    assert!(r.contains("+ 2  B") && r.contains("+ 3  B2"), "out: {r}");
    assert!(r.contains("新行号 2-3"), "out: {r}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn duplicate_insert_warns() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "impl Foo {\n    /// 创建空索引\n    pub fn new() -> Self {\n}\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 模拟 LLM 复制错误：插入文本把插入点后的行也抄了进去
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: None,
                end_line: None,
                insert_before: Some(2),
                text: "    /// 创建空索引\n    pub fn new() -> Self {".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("⚠️ 警告"), "out: {r}");
    assert!(r.contains("重复插入"), "out: {r}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn legit_duplicate_insert_does_not_warn() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "a\nb\nc\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 插入点后首行是 c，插入文本末行是 b → 不触发警告
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: None,
                end_line: None,
                insert_before: Some(3),
                text: "x\nb".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(!r.contains("⚠️"), "out: {r}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn edit_empty_file_via_append() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    tool.call(EditFileArgs {
        path: "a.txt".to_string(),
        edits: vec![EditOp {
            start_line: None,
            end_line: None,
            insert_before: None,
            text: "hello".to_string(),
        }],
        dry_run: None,
        diff_context: None,
    })
    .await
    .unwrap();
    assert_eq!(read(&p).await, "hello\n");
    std::fs::remove_dir_all(&dir).ok();
}
#[tokio::test]
async fn dry_run_previews_without_writing() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\nb\nc\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: None,
                insert_before: None,
                text: "B".to_string(),
            }],
            dry_run: Some(true),
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("[预览]"), "out: {r}");
    assert!(r.contains("未写入磁盘"), "out: {r}");
    assert!(r.contains("+ 2  B"), "out: {r}");
    // 文件内容未被改变
    assert_eq!(read(&p).await, "a\nb\nc\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn noop_replace_warns() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "a\nb\nc\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 替换文本与原内容完全相同 → 应警告未产生实际变更
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: None,
                insert_before: None,
                text: "b".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("未产生实际变更"), "out: {r}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn whitespace_only_replace_hints() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "fn main() {\n    let x = 1;\n}\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 只改缩进：4 空格 → 2 空格，内容相同 → 应提示仅有空白差异
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: None,
                insert_before: None,
                text: "  let x = 1;".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("仅有空白/缩进差异"), "out: {r}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn diff_context_shows_surrounding_lines() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "a\nb\nc\nd\ne\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 默认 diff_context=1：diff 含上下文行（· 标记）
    // 用 dry_run 避免修改文件，保证第二次调用仍在原文件上操作
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(3),
                end_line: None,
                insert_before: None,
                text: "C".to_string(),
            }],
            dry_run: Some(true),
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("· 2  b"), "out: {r}");
    assert!(r.contains("- 3  c"), "out: {r}");
    assert!(r.contains("+ 3  C"), "out: {r}");
    assert!(r.contains("· 4  d"), "out: {r}");

    // diff_context=0：不显示上下文行（验证上下文行号 2 和 4 不出现）
    // 注意：不能简单断言 !contains("·")，因为"变更明细（· 上下文...）"
    // 标题里也含 · 字符；改为检查上下文行号格式 "· 2" / "· 4" 不出现
    let r2 = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(3),
                end_line: None,
                insert_before: None,
                text: "C".to_string(),
            }],
            dry_run: Some(true),
            diff_context: Some(0),
        })
        .await
        .unwrap();
    assert!(!r2.contains("· 2"), "out: {r2}");
    assert!(!r2.contains("· 4"), "out: {r2}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn line_calibration_warns_on_mismatch() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "a\nb\nc\nd\ne\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 声明替换 3 行（2-4），实际写入 1 行 → 应触发行数校准警告
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: Some(4),
                insert_before: None,
                text: "X".to_string(),
            }],
            dry_run: Some(true),
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("行数校准"), "out: {r}");
    assert!(r.contains("声明替换 3 行"), "out: {r}");
    assert!(r.contains("实际写入 1 行"), "out: {r}");
    assert!(r.contains("-2"), "out: {r}"); // delta = 1 - 3 = -2
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn line_calibration_no_warning_when_matching() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "a\nb\nc\nd\ne\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 声明替换 2 行（2-3），实际写入 2 行 → 不应触发行数校准警告
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: Some(3),
                insert_before: None,
                text: "X\nY".to_string(),
            }],
            dry_run: Some(true),
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(!r.contains("行数校准"), "out: {r}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn line_calibration_warns_on_expansion() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "a\nb\nc\nd\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    // 声明替换 1 行（2），实际写入 3 行 → 应触发行数校准警告（+2）
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: None,
                insert_before: None,
                text: "X\nY\nZ".to_string(),
            }],
            dry_run: Some(true),
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("行数校准"), "out: {r}");
    assert!(r.contains("声明替换 1 行"), "out: {r}");
    assert!(r.contains("实际写入 3 行"), "out: {r}");
    assert!(r.contains("+2"), "out: {r}"); // delta = 3 - 1 = +2
    std::fs::remove_dir_all(&dir).ok();
}
