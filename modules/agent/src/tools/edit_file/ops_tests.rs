//! edit_file 操作类测试：替换 / 插入 / 追加 / 删除 / EOL 保留 / 多操作 / 校验。

use super::tests_common::{read, setup_file, tmp_dir};
use super::*;
use rig_core::tool::Tool;

#[tokio::test]
async fn replace_single_line() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "line1\nline2\nline3\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: None,
                insert_before: None,
                text: "<content>CHANGED</content>".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("替换第 2 行"));
    assert_eq!(read(&p).await, "line1\nCHANGED\nline3\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn replace_range_with_multiline() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\nb\nc\nd\ne\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(2),
                end_line: Some(4),
                insert_before: None,
                text: "<content>X\nY</content>".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("替换第 2-4 行 → 2 行新文本"));
    assert_eq!(read(&p).await, "a\nX\nY\ne\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn delete_line_with_empty_text() {
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
                text: String::new(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("删除原第 2-2 行"));
    assert_eq!(read(&p).await, "a\nc\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn insert_before_line() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\nb\nc\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: None,
                end_line: None,
                insert_before: Some(2),
                text: "<content>INS</content>".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("在第 2 行前插入 1 行新文本"));
    assert_eq!(read(&p).await, "a\nINS\nb\nc\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn append_writes_new_last_line() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\nb\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: None,
                end_line: None,
                insert_before: None,
                text: "<content>c\nd</content>".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await
        .unwrap();
    assert!(r.contains("在文件末尾追加 2 行新文本"));
    assert_eq!(read(&p).await, "a\nb\nc\nd\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn append_to_file_without_trailing_newline() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\nb").await; // 无结尾换行
    let tool = EditFileTool::with_cwd(dir.clone());
    tool.call(EditFileArgs {
        path: "a.txt".to_string(),
        edits: vec![EditOp {
            start_line: None,
            end_line: None,
            insert_before: None,
            text: "c".to_string(),
        }],
        dry_run: None,
        diff_context: None,
    })
    .await
    .unwrap();
    // 追加内容作为"新行"写入，需补换行
    assert_eq!(read(&p).await, "a\nb\nc\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn preserve_missing_trailing_newline_without_append() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\nb").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    tool.call(EditFileArgs {
        path: "a.txt".to_string(),
        edits: vec![EditOp {
            start_line: Some(2),
            end_line: None,
            insert_before: None,
            text: "B".to_string(),
        }],
        dry_run: None,
        diff_context: None,
    })
    .await
    .unwrap();
    assert_eq!(read(&p).await, "a\nB"); // 保留无结尾换行
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn preserve_crlf_style() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\r\nb\r\nc\r\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    tool.call(EditFileArgs {
        path: "a.txt".to_string(),
        edits: vec![EditOp {
            start_line: Some(2),
            end_line: None,
            insert_before: None,
            text: "B".to_string(),
        }],
        dry_run: None,
        diff_context: None,
    })
    .await
    .unwrap();
    assert_eq!(read(&p).await, "a\r\nB\r\nc\r\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn multiple_edits_in_one_call() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "1\n2\n3\n4\n5\n6\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    tool.call(EditFileArgs {
        path: "a.txt".to_string(),
        edits: vec![
            EditOp { start_line: Some(2), end_line: None, insert_before: None, text: "two".to_string() },
            EditOp { start_line: None, end_line: None, insert_before: Some(4), text: "inserted".to_string() },
            EditOp { start_line: Some(6), end_line: Some(6), insert_before: None, text: "six".to_string() },
            EditOp { start_line: None, end_line: None, insert_before: None, text: "seven".to_string() },
        ],
        dry_run: None,
        diff_context: None,
    })
    .await
    .unwrap();
    assert_eq!(read(&p).await, "1\ntwo\n3\ninserted\n4\n5\nsix\nseven\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn same_position_inserts_keep_array_order() {
    let dir = tmp_dir();
    let p = setup_file(&dir, "a.txt", "a\nb\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    tool.call(EditFileArgs {
        path: "a.txt".to_string(),
        edits: vec![
            EditOp { start_line: None, end_line: None, insert_before: Some(1), text: "A".to_string() },
            EditOp { start_line: None, end_line: None, insert_before: Some(1), text: "B".to_string() },
        ],
        dry_run: None,
        diff_context: None,
    })
    .await
    .unwrap();
    assert_eq!(read(&p).await, "A\nB\na\nb\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn overlapping_ranges_rejected() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "1\n2\n3\n4\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![
                EditOp { start_line: Some(1), end_line: Some(3), insert_before: None, text: "x".to_string() },
                EditOp { start_line: Some(2), end_line: Some(4), insert_before: None, text: "y".to_string() },
            ],
            dry_run: None,
            diff_context: None,
        })
        .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("重叠"));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn out_of_range_rejected() {
    let dir = tmp_dir();
    setup_file(&dir, "a.txt", "1\n2\n").await;
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "a.txt".to_string(),
            edits: vec![EditOp {
                start_line: Some(9),
                end_line: None,
                insert_before: None,
                text: "x".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("超出"));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn missing_file_rejected() {
    let dir = tmp_dir();
    let tool = EditFileTool::with_cwd(dir.clone());
    let r = tool
        .call(EditFileArgs {
            path: "nope.txt".to_string(),
            edits: vec![EditOp {
                start_line: None,
                end_line: None,
                insert_before: None,
                text: "x".to_string(),
            }],
            dry_run: None,
            diff_context: None,
        })
        .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("write_file"));
    std::fs::remove_dir_all(&dir).ok();
}
