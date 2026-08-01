//! 应用数据目录与 ASR 相关文件路径工具函数。
//!
//! 所有函数基于 `appdata_root()`（`<app_data_dir>/effisuite`）派生子路径，
//! 供 ASR 存储层定位音频文件、记录索引与转写文本。
//!
//! 设计要点：
//! - 与 `tauriFront/src-tauri/src/paths.rs` 风格一致，均基于 `dirs` crate
//! - 目录型函数在首次调用时通过 `std::fs::create_dir_all` 创建目录，幂等安全
//! - 文件型函数仅返回路径，父目录由对应目录型函数确保存在

use std::path::PathBuf;

/// appdata 根目录：`<app_data_dir>/effisuite`
///
/// 与 `tauriFront` 的 `appdata_root()` 保持一致，确保 core 与前端
/// 读写同一数据目录。回退到 `config_dir`（macOS/Linux）或当前目录。
fn appdata_root() -> PathBuf {
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("effisuite")
}

/// ASR 存储根目录：`<appdata>/effisuite/asr/`
///
/// 首次调用时创建目录（含父目录），幂等安全。
pub fn asr_dir() -> PathBuf {
    let dir = appdata_root().join("asr");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// ASR 原始音频文件目录：`<appdata>/effisuite/asr/audio/`
///
/// 首次调用时创建目录，幂等安全。
pub fn asr_audio_dir() -> PathBuf {
    let dir = asr_dir().join("audio");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// ASR 记录元数据索引文件：`<appdata>/effisuite/asr/records.json`
///
/// 父目录由 [`asr_dir`] 确保存在。
pub fn asr_records_path() -> PathBuf {
    asr_dir().join("records.json")
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asr_dir_ends_with_asr() {
        let dir = asr_dir();
        assert!(dir.ends_with("asr"));
        assert!(dir.is_dir(), "asr_dir should exist after call");
    }

    #[test]
    fn asr_audio_dir_ends_with_audio() {
        let dir = asr_audio_dir();
        assert!(dir.ends_with("audio"));
        assert!(dir.is_dir(), "asr_audio_dir should exist after call");
    }

    #[test]
    fn asr_records_path_ends_with_records_json() {
        let path = asr_records_path();
        assert!(path.ends_with("records.json"));
    }

    #[test]
    fn asr_audio_dir_is_subdir_of_asr_dir() {
        let audio = asr_audio_dir();
        let asr = asr_dir();
        assert_eq!(audio.parent(), Some(asr.as_path()));
    }
}
