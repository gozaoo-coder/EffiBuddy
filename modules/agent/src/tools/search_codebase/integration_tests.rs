use super::*;

// ============ 端到端集成测试 ============

fn tmp_dir() -> PathBuf {
    std::env::temp_dir()
        .join(format!("effisuite-search-codebase-{}", uuid::Uuid::new_v4()))
}

#[tokio::test]
async fn search_finds_relevant_code() {
    let dir = tmp_dir();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/auth.rs"),
        "pub fn verify_token(token: &str) -> Result<Claims> {\n    let key = get_secret_key();\n    let claims = decode::<Claims>(token, &key)?;\n    Ok(claims)\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "verify token".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("找到 1 个相关代码块"), "out: {out}");
    assert!(out.contains("src/auth.rs"), "out: {out}");
    assert!(out.contains("verify_token"), "out: {out}");
    // 不相关的 main.rs 不应出现
    assert!(!out.contains("src/main.rs"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_returns_block_with_line_numbers() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("auth.rs"),
        "use std::io;\n\npub fn verify_token(token: &str) -> bool {\n    let key = \"secret\";\n    token == key\n}\n",
    )
    .unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "authentication token verify".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await
        .unwrap();

    // 应包含带行号的代码块（行号与 read_file 对齐：右对齐 + 两空格）
    assert!(out.contains("3  pub fn verify_token"), "out: {out}");
    assert!(out.contains("4      let key = \"secret\";"), "out: {out}");
    assert!(out.contains("得分:"), "out: {out}");
    assert!(out.contains("匹配关键词:"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_skips_non_code_files() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // .txt 文件不在 CODE_EXTS 中
    std::fs::write(dir.join("auth.txt"), "verify_token secret\n").unwrap();
    // .lock 文件也不在
    std::fs::write(dir.join("Cargo.lock"), "auth verify\n").unwrap();
    std::fs::write(dir.join("auth.rs"), "pub fn auth() {}\n").unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "auth".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("auth.rs"), "out: {out}");
    assert!(!out.contains("auth.txt"), "out: {out}");
    assert!(!out.contains("Cargo.lock"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_skips_generated_dirs() {
    let dir = tmp_dir();
    std::fs::create_dir_all(dir.join("target/debug")).unwrap();
    std::fs::write(dir.join("target/debug/out.rs"), "auth verify\n").unwrap();
    std::fs::write(dir.join("main.rs"), "pub fn auth() {}\n").unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "auth".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("main.rs"), "out: {out}");
    assert!(!out.contains("target"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_respects_max_results() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // 创建 5 个匹配文件
    for i in 0..5 {
        std::fs::write(
            dir.join(format!("auth{i}.rs")),
            format!("pub fn auth_verify_{i}() -> bool {{ true }}\n"),
        )
        .unwrap();
    }

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "auth verify".to_string(),
            target_directories: None,
            max_results: Some(3),
        })
        .await
        .unwrap();

    assert!(out.contains("找到 3 个相关代码块"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_no_match_returns_message() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "nonexistent concept xyz".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("未找到"), "out: {out}");
    assert!(out.contains("nonexistent concept xyz"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_rejects_empty_query() {
    let tool = SearchCodebaseTool::new();
    let r = tool
        .call(SearchCodebaseArgs {
            query: "   ".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("query"));
}

#[tokio::test]
async fn search_rejects_stopwords_only_query() {
    let tool = SearchCodebaseTool::new();
    let r = tool
        .call(SearchCodebaseArgs {
            query: "the a an is how".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("无法从查询"));
}

#[tokio::test]
async fn search_target_directories() {
    let dir = tmp_dir();
    std::fs::create_dir_all(dir.join("sub1")).unwrap();
    std::fs::create_dir_all(dir.join("sub2")).unwrap();
    std::fs::write(dir.join("sub1/auth.rs"), "pub fn auth() {}\n").unwrap();
    std::fs::write(dir.join("sub2/other.rs"), "pub fn other() {}\n").unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "auth".to_string(),
            target_directories: Some(vec!["sub1".to_string()]),
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("sub1/auth.rs"), "out: {out}");
    assert!(!out.contains("sub2/other.rs"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_multiple_target_directories() {
    let dir = tmp_dir();
    std::fs::create_dir_all(dir.join("sub1")).unwrap();
    std::fs::create_dir_all(dir.join("sub2")).unwrap();
    std::fs::write(dir.join("sub1/auth.rs"), "pub fn auth() {}\n").unwrap();
    std::fs::write(dir.join("sub2/token.rs"), "pub fn token() {}\n").unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "auth token".to_string(),
            target_directories: Some(vec!["sub1".to_string(), "sub2".to_string()]),
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("sub1/auth.rs"), "out: {out}");
    assert!(out.contains("sub2/token.rs"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_skips_binary_files() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // 含 NUL 字节的"二进制"代码文件（按扩展名是 .rs，但内容是二进制）
    std::fs::write(dir.join("bin.rs"), b"\x00\x01\x02auth\x00\x03").unwrap();
    std::fs::write(dir.join("real.rs"), "pub fn auth() {}\n").unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "auth".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("real.rs"), "out: {out}");
    assert!(!out.contains("bin.rs"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn search_chinese_query() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("user.rs"),
        "pub struct User {\n    pub name: String,\n    pub id: u64,\n}\n\npub fn create_user(name: &str) -> User {\n    User { name: name.to_string(), id: 0 }\n}\n",
    )
    .unwrap();

    let tool = SearchCodebaseTool::with_cwd(dir.clone());
    let out = tool
        .call(SearchCodebaseArgs {
            query: "user create".to_string(),
            target_directories: None,
            max_results: None,
        })
        .await
        .unwrap();

    assert!(out.contains("user.rs"), "out: {out}");
    assert!(out.contains("create_user"), "out: {out}");

    std::fs::remove_dir_all(&dir).ok();
}
