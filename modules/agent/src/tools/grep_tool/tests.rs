use std::path::PathBuf;

use super::*;

fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("effisuite-grep-test-{}", uuid::Uuid::new_v4()))
}

#[tokio::test]
async fn grep_basic_regex_match() {
    let dir = tmp_dir();
    std::fs::create_dir_all(dir.join("src/nested")).unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        "fn main() {\n    let x = 1;\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/nested/lib.rs"),
        "pub fn helper() {\n    return 42;\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "no match here\n").unwrap();
    // 生成目录应被跳过
    std::fs::create_dir_all(dir.join("target/debug")).unwrap();
    std::fs::write(dir.join("target/debug/out.rs"), "fn skipped() {}\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());
    let out = tool
        .call(GrepArgs {
            pattern: r"fn \w+".to_string(),
            path: None,
            output_mode: None,
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();

    assert!(out.contains("2 个文件命中"), "out: {out}");
    assert!(out.contains("path: src/main.rs"), "out: {out}");
    assert!(out.contains("fn main() {"), "out: {out}");
    assert!(out.contains("path: src/nested/lib.rs"), "out: {out}");
    assert!(out.contains("pub fn helper() {"), "out: {out}");
    // README 不含 fn 定义
    assert!(!out.contains("README.md"), "out: {out}");
    // target 目录被跳过
    assert!(!out.contains("target"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_case_insensitive_default() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "Hello World\nhello rust\nHELLO there\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());

    // 默认不区分大小写：三行都命中
    let out = tool
        .call(GrepArgs {
            pattern: "hello".to_string(),
            path: None,
            output_mode: None,
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();
    assert!(out.contains("命中 3 处"), "out: {out}");
    assert!(
        out.contains("Hello World") && out.contains("hello rust") && out.contains("HELLO there"),
        "out: {out}"
    );

    // 区分大小写：只有 hello rust 命中
    let out = tool
        .call(GrepArgs {
            pattern: "hello".to_string(),
            path: None,
            output_mode: None,
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: Some(true),
            multiline: None,
        })
        .await
        .unwrap();
    assert!(out.contains("命中 1 处"), "out: {out}");
    assert!(out.contains("hello rust"), "out: {out}");
    assert!(!out.contains("Hello World"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_files_with_matches_mode() {
    let dir = tmp_dir();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.rs"), "fn alpha() {}\n").unwrap();
    std::fs::write(dir.join("sub/b.rs"), "fn beta() {}\nno match\n").unwrap();
    std::fs::write(dir.join("c.txt"), "nothing\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());
    let out = tool
        .call(GrepArgs {
            pattern: r"fn \w+".to_string(),
            path: None,
            output_mode: Some("files_with_matches".to_string()),
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();

    assert!(out.contains("2 个文件命中"), "out: {out}");
    assert!(out.contains("a.rs"), "out: {out}");
    assert!(out.contains("sub/b.rs"), "out: {out}");
    assert!(!out.contains("c.txt"), "out: {out}");
    // files_with_matches 不应输出行内容
    assert!(!out.contains("fn alpha"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_count_mode() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "foo\nfoo bar\nbaz\nfoo\n").unwrap();
    std::fs::write(dir.join("b.txt"), "baz\nqux\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());
    let out = tool
        .call(GrepArgs {
            pattern: "foo".to_string(),
            path: None,
            output_mode: Some("count".to_string()),
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();

    assert!(out.contains("a.txt: 3 matches"), "out: {out}");
    // b.txt 无命中，不应出现
    assert!(!out.contains("b.txt"), "out: {out}");
    assert!(out.contains("1 个文件命中"), "out: {out}");
    assert!(out.contains("3 处"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_glob_filter_rs_only() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.rs"), "fn rust_fn() {}\n").unwrap();
    std::fs::write(dir.join("b.go"), "fn go_fn() {}\n").unwrap();
    std::fs::write(dir.join("c.rs"), "fn another_fn() {}\n").unwrap();
    std::fs::write(dir.join("d.txt"), "fn txt_fn() {}\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());
    let out = tool
        .call(GrepArgs {
            pattern: r"fn \w+".to_string(),
            path: None,
            output_mode: Some("files_with_matches".to_string()),
            glob: Some("*.rs".to_string()),
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();

    assert!(out.contains("2 个文件命中"), "out: {out}");
    assert!(out.contains("a.rs"), "out: {out}");
    assert!(out.contains("c.rs"), "out: {out}");
    // .go 和 .txt 被过滤掉
    assert!(!out.contains("b.go"), "out: {out}");
    assert!(!out.contains("d.txt"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_context_lines_display() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "l1\nl2\ntokio hit\nl4\nl5\n").unwrap();
    let tool = GrepTool::with_cwd(dir.clone());
    let out = tool
        .call(GrepArgs {
            pattern: "tokio".to_string(),
            path: None,
            output_mode: None,
            glob: None,
            context: Some(1),
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();
    // 上下文行以 · 前缀标记，命中行保持原格式；path 行附上文件总行数
    assert!(out.contains("· 2  l2"), "out: {out}");
    assert!(out.contains("3  tokio hit"), "out: {out}");
    assert!(out.contains("· 4  l4"), "out: {out}");
    assert!(out.contains("a.txt（共 5 行）"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_no_match_returns_message() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "nothing here\njust text\n").unwrap();
    let tool = GrepTool::with_cwd(dir.clone());

    let out = tool
        .call(GrepArgs {
            pattern: r"\d{4}-\d{2}-\d{2}".to_string(),
            path: None,
            output_mode: None,
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();
    assert!(out.contains("未找到"), "out: {out}");
    assert!(out.contains("共扫描"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_invalid_regex_errors() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let tool = GrepTool::with_cwd(dir.clone());

    // 未闭合的分组
    let r = tool
        .call(GrepArgs {
            pattern: r"fn (\w+".to_string(),
            path: None,
            output_mode: None,
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await;
    assert!(r.is_err());
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("正则表达式编译失败"), "msg: {msg}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_invalid_output_mode_errors() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let tool = GrepTool::with_cwd(dir.clone());

    let r = tool
        .call(GrepArgs {
            pattern: "foo".to_string(),
            path: None,
            output_mode: Some("bogus_mode".to_string()),
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("output_mode"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_skips_binary_files() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // 含 NUL 字节的"二进制"文件
    std::fs::write(dir.join("bin.dat"), b"\x00\x01\x02tokio\x00\x03").unwrap();
    std::fs::write(dir.join("a.txt"), "tokio text\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());
    let out = tool
        .call(GrepArgs {
            pattern: "tokio".to_string(),
            path: None,
            output_mode: None,
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();
    assert!(out.contains("path: a.txt"), "out: {out}");
    assert!(!out.contains("bin.dat"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_respects_subdir_path() {
    let dir = tmp_dir();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("root.txt"), "tokio root\n").unwrap();
    std::fs::write(dir.join("sub/inner.txt"), "tokio inner\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());
    let out = tool
        .call(GrepArgs {
            pattern: "tokio".to_string(),
            path: Some("sub".to_string()),
            output_mode: None,
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();
    // 路径基准相对工作区 cwd：path=sub 时返回 "sub/inner.txt" 而非 "inner.txt"，
    // 保证 LLM 回传 read_file/edit_file 时路径可解析
    assert!(out.contains("path: sub/inner.txt"), "out: {out}");
    assert!(!out.contains("root.txt"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn grep_multiline_cross_line_match() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // 跨行匹配：foo 后跟换行再跟 bar
    std::fs::write(dir.join("a.txt"), "foo\nbar\nbaz\n").unwrap();
    std::fs::write(dir.join("b.txt"), "foo bar\nsingle\n").unwrap();

    let tool = GrepTool::with_cwd(dir.clone());
    // multiline=true：foo\nbar 可跨行匹配
    let out = tool
        .call(GrepArgs {
            pattern: "foo\\nbar".to_string(),
            path: None,
            output_mode: Some("files_with_matches".to_string()),
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: Some(true),
        })
        .await
        .unwrap();
    // a.txt 含跨行的 foo\nbar，b.txt 不含（其 foo bar 在同一行无换行）
    assert!(out.contains("a.txt"), "out: {out}");
    assert!(!out.contains("b.txt"), "out: {out}");

    // 同样的正则在逐行模式下匹配不到（每行单独匹配，无行内含 foo\nbar）
    let out2 = tool
        .call(GrepArgs {
            pattern: "foo\\nbar".to_string(),
            path: None,
            output_mode: Some("files_with_matches".to_string()),
            glob: None,
            context: None,
            max_matches: None,
            case_sensitive: None,
            multiline: None,
        })
        .await
        .unwrap();
    assert!(out2.contains("未找到"), "out: {out2}");

    std::fs::remove_dir_all(&dir).ok();
}

// ============ 纯函数单元测试 ============

#[test]
fn glob_match_basic() {
    assert!(glob_match("*.rs", "main.rs"));
    assert!(glob_match("*.rs", "a.rs"));
    assert!(!glob_match("*.rs", "main.go"));
    assert!(!glob_match("*.rs", "rust"));
    assert!(glob_match("*", "anything"));
    assert!(glob_match("*", ""));
    assert!(glob_match("a?c", "abc"));
    assert!(!glob_match("a?c", "ac"));
    assert!(glob_match("*.test.ts", "foo.test.ts"));
    assert!(!glob_match("*.test.ts", "foo.test.js"));
    // 多段 *
    assert!(glob_match("*main*", "src/main.rs"));
    assert!(glob_match("src/*.rs", "src/main.rs"));
    assert!(!glob_match("src/*.rs", "main.rs"));
}

#[test]
fn line_index_of_basic() {
    // 单行文本，line_starts = [0]
    let starts = vec![0usize];
    assert_eq!(line_index_of(&starts, 0), 0);

    // "ab\ncd\nef" → line_starts = [0, 3, 6]
    let starts = vec![0, 3, 6];
    assert_eq!(line_index_of(&starts, 0), 0);
    assert_eq!(line_index_of(&starts, 2), 0);
    assert_eq!(line_index_of(&starts, 3), 1);
    assert_eq!(line_index_of(&starts, 5), 1);
    assert_eq!(line_index_of(&starts, 6), 2);
    assert_eq!(line_index_of(&starts, 8), 2);
}

#[test]
fn compute_line_starts_correct() {
    let text = "ab\ncd\nef";
    let starts = compute_line_starts(text);
    assert_eq!(starts, vec![0, 3, 6]);

    let empty = "";
    assert_eq!(compute_line_starts(empty), vec![0]);

    let no_newline = "abc";
    assert_eq!(compute_line_starts(no_newline), vec![0]);

    let trailing = "a\nb\n";
    assert_eq!(compute_line_starts(trailing), vec![0, 2, 4]);
}

#[test]
fn is_binary_detection() {
    assert!(!is_binary(b"plain text file"));
    assert!(!is_binary(b""));
    assert!(is_binary(b"\x00binary"));
    assert!(is_binary(b"text\x00more"));
    // NUL 在 8KB 之外不算二进制（探测前 8KB）
    let mut big = vec![b'a'; 9000];
    big.push(0);
    assert!(!is_binary(&big));
}
