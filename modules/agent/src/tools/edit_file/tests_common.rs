//! edit_file 测试共用辅助函数。

use std::path::PathBuf;

pub(super) fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("effisuite-edit-test-{}", uuid::Uuid::new_v4()))
}

pub(super) async fn setup_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(&p, content).unwrap();
    p
}

pub(super) async fn read(p: &PathBuf) -> String {
    tokio::fs::read_to_string(p).await.unwrap()
}
