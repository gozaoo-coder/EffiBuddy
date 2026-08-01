use super::constants::MAX_BLOCK_LINES;
use super::scorer::is_definition_line;
use super::*;

/// 测试辅助：构造原文 + 小写行后调用 score_file
fn score_for_test(
    lines: &[&str],
    keywords: &[String],
) -> (f64, Vec<String>, Vec<usize>) {
    let lower: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();
    let lower_refs: Vec<&str> = lower.iter().map(|s| s.as_str()).collect();
    score_file(lines, &lower_refs, keywords)
}

// ============ extract_keywords ============

#[test]
fn extract_keywords_basic() {
    let kws = extract_keywords("how does authentication work");
    assert!(kws.iter().any(|k| k == "authentication"));
    assert!(kws.iter().any(|k| k == "work"));
    assert!(!kws.iter().any(|k| k == "how")); // 停用词
    assert!(!kws.iter().any(|k| k == "does")); // 停用词
}

#[test]
fn extract_keywords_chinese() {
    // 中文按字符分词，用空格分隔词语
    let kws = extract_keywords("处理 用户 登录 逻辑");
    assert!(kws.iter().any(|k| k == "处理"));
    assert!(kws.iter().any(|k| k == "用户"));
    assert!(kws.iter().any(|k| k == "登录"));
    assert!(kws.iter().any(|k| k == "逻辑"));
}

#[test]
fn extract_keywords_chinese_single_chars() {
    // 无空格的中文按单字分词
    let kws = extract_keywords("处理用户登录的逻辑");
    // "的" 是停用词，应被过滤
    assert!(!kws.iter().any(|k| k == "的"));
    // 其他单字应保留
    assert!(kws.iter().any(|k| k == "处"));
    assert!(kws.iter().any(|k| k == "理"));
    assert!(kws.iter().any(|k| k == "登"));
    assert!(kws.iter().any(|k| k == "录"));
}

#[test]
fn extract_keywords_dedup() {
    let kws = extract_keywords("auth auth auth");
    assert_eq!(kws, vec!["auth"]);
}

#[test]
fn extract_keywords_strips_short_english() {
    // 单字符英文应被过滤
    let kws = extract_keywords("a b auth");
    assert_eq!(kws, vec!["auth"]);
}

#[test]
fn extract_keywords_empty_query() {
    assert!(extract_keywords("").is_empty());
    assert!(extract_keywords("   ").is_empty());
    assert!(extract_keywords("the a an is").is_empty());
}

// ============ is_definition_line ============

#[test]
fn is_definition_line_detects_rust() {
    assert!(is_definition_line("pub fn verify_token(token: &str) -> Result<Claims> {"));
    assert!(is_definition_line("struct User {"));
    assert!(is_definition_line("enum Role {"));
    assert!(is_definition_line("impl User {"));
    assert!(is_definition_line("async fn fetch() {"));
    assert!(is_definition_line("pub(crate) fn helper() {"));
}

#[test]
fn is_definition_line_detects_python() {
    assert!(is_definition_line("def verify_token(token):"));
    assert!(is_definition_line("class User:"));
    assert!(is_definition_line("async def fetch():"));
}

#[test]
fn is_definition_line_detects_other_langs() {
    assert!(is_definition_line("function foo() {"));
    assert!(is_definition_line("func bar() {"));
    assert!(is_definition_line("interface Baz {"));
    assert!(is_definition_line("type Quux = {"));
}

#[test]
fn is_definition_line_ignores_comments() {
    assert!(!is_definition_line("// fn commented_out() {"));
    assert!(!is_definition_line("# def commented():"));
    assert!(!is_definition_line("/* class Old { */"));
    assert!(!is_definition_line("-- fn sql_comment"));
}

#[test]
fn is_definition_line_ignores_empty() {
    assert!(!is_definition_line(""));
    assert!(!is_definition_line("   "));
}

// ============ is_code_file ============

#[test]
fn is_code_file_recognizes_extensions() {
    assert!(is_code_file("main.rs"));
    assert!(is_code_file("app.py"));
    assert!(is_code_file("index.ts"));
    assert!(is_code_file("Component.tsx"));
    assert!(is_code_file("main.go"));
    assert!(is_code_file("Cargo.toml"));
    assert!(is_code_file("README.md"));
}

#[test]
fn is_code_file_rejects_non_code() {
    assert!(!is_code_file("auth.txt"));
    assert!(!is_code_file("Cargo.lock"));
    assert!(!is_code_file("image.png"));
    assert!(!is_code_file("archive.tar.gz")); // ext = "gz"，不在列表
    assert!(!is_code_file("noext"));
}

// ============ find_code_block ============

#[test]
fn find_code_block_expands_to_function() {
    let lines: Vec<&str> = vec![
        "use std::io;",                                            // 1
        "",                                                        // 2
        "pub fn verify_token(token: &str) -> Result<Claims> {",   // 3
        "    let key = get_secret_key();",                        // 4
        "    let claims = decode::<Claims>(token, &key)?;",       // 5
        "    Ok(claims)",                                          // 6
        "}",                                                       // 7
        "",                                                        // 8
        "pub fn other_function() {",                              // 9
        "    todo!()",                                             // 10
        "}",                                                       // 11
    ];
    // 命中第 4 行 → 应扩展到第 3-7 行（完整函数）
    let (start, end) = find_code_block(&lines, 4);
    assert_eq!(start, 3);
    assert_eq!(end, 7);
}

#[test]
fn find_code_block_falls_back_when_no_def() {
    let lines: Vec<&str> = vec![
        "let a = 1;",  // 1
        "let b = 2;",  // 2
        "let c = 3;",  // 3
        "let d = 4;",  // 4
        "let e = 5;",  // 5
        "let f = 6;",  // 6
        "let g = 7;",  // 7
        "let h = 8;",  // 8
        "let i = 9;",  // 9
        "let j = 10;", // 10
        "let k = 11;", // 11
    ];
    // 命中第 6 行，无定义行 → 以 6 为中心扩展 ±5 = 1-11
    let (start, end) = find_code_block(&lines, 6);
    assert_eq!(start, 1);
    assert_eq!(end, 11);
}

#[test]
fn find_code_block_python_stops_at_next_def() {
    let lines: Vec<&str> = vec![
        "def verify_token(token):",      // 1
        "    key = get_secret()",        // 2
        "    return token == key",       // 3
        "",                              // 4
        "def other_function():",         // 5
        "    pass",                      // 6
    ];
    // 命中第 2 行 → 从 def 开始到下一个 def 之前（第 1-4 行）
    let (start, end) = find_code_block(&lines, 2);
    assert_eq!(start, 1);
    assert_eq!(end, 4);
}

#[test]
fn find_code_block_clamps_to_max_block_lines() {
    // 一个超长函数（无大括号匹配的边界情况）
    let owned: Vec<String> = (0..200).map(|i| format!("    let v{i} = {i};")).collect();
    let lines: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    // 命中第 100 行，无定义行，无大括号 → 扩展到 start+MAX_BLOCK_LINES
    let (start, end) = find_code_block(&lines, 100);
    assert!(end - start + 1 <= MAX_BLOCK_LINES, "block too large: {}", end - start + 1);
}

// ============ score_file ============

#[test]
fn score_file_ranks_definition_higher() {
    let def_lines: Vec<&str> = vec![
        "pub fn auth_verify(token: &str) -> bool {",
        "    let key = \"secret\";",
        "    token == key",
        "}",
    ];
    let non_def_lines: Vec<&str> = vec![
        "    let s = \"auth_verify\";",
        "    let t = \"hello\";",
        "    let u = \"world\";",
        "    let v = \"foo\";",
    ];
    let keywords = vec!["auth".to_string(), "verify".to_string()];
    let (def_score, _, _) = score_for_test(&def_lines, &keywords);
    let (non_def_score, _, _) = score_for_test(&non_def_lines, &keywords);
    assert!(
        def_score > non_def_score,
        "def: {def_score}, non_def: {non_def_score}"
    );
}

#[test]
fn score_file_rewards_keyword_coverage() {
    let lines: Vec<&str> = vec![
        "pub fn auth_verify(token: &str) -> bool {",
        "    let key = get_secret();",
        "    token == key",
        "}",
    ];
    let single_kw = vec!["auth".to_string()];
    let multi_kw = vec!["auth".to_string(), "verify".to_string()];
    let (single_score, _, _) = score_for_test(&lines, &single_kw);
    let (multi_score, _, _) = score_for_test(&lines, &multi_kw);
    // 命中更多关键词时，覆盖率奖励让得分更高
    assert!(
        multi_score > single_score,
        "single: {single_score}, multi: {multi_score}"
    );
}

#[test]
fn score_file_empty_inputs() {
    let empty: Vec<&str> = vec![];
    let kws = vec!["auth".to_string()];
    let (s, m, h) = score_for_test(&empty, &kws);
    assert_eq!(s, 0.0);
    assert!(m.is_empty());
    assert!(h.is_empty());

    let lines = vec!["pub fn auth() {}"];
    let empty_kws: Vec<String> = vec![];
    let (s, _, _) = score_for_test(&lines, &empty_kws);
    assert_eq!(s, 0.0);
}

#[test]
fn score_file_no_match_returns_zero() {
    let lines: Vec<&str> = vec!["fn main() {", "    println!(\"hi\");", "}"];
    let kws = vec!["auth".to_string(), "token".to_string()];
    let (s, m, h) = score_for_test(&lines, &kws);
    assert_eq!(s, 0.0);
    assert!(m.is_empty());
    assert!(h.is_empty());
}
