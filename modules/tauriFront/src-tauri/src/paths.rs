//! 应用数据目录与文件路径工具函数。
//!
//! 所有函数基于 `appdata_root()`（`<app_data_dir>/effisuite`）派生子路径，
//! 供配置持久化、会话/技能/插件/压缩/定时任务存储与附件落盘使用。

/// appdata 根目录：`<app_data_dir>/effisuite`
pub(crate) fn appdata_root() -> std::path::PathBuf {
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("effisuite")
}

/// 配置文件路径：`<appdata>/config.json`
pub(crate) fn config_path() -> std::path::PathBuf {
    appdata_root().join("config.json")
}

/// 会话存储目录：`<appdata>/conversations`
pub(crate) fn conversations_dir() -> std::path::PathBuf {
    appdata_root().join("conversations")
}

/// 技能存储目录：`<appdata>/skills`
pub(crate) fn skills_dir() -> std::path::PathBuf {
    appdata_root().join("skills")
}

/// 插件存储目录：`<appdata>/plugins`
pub(crate) fn plugins_dir() -> std::path::PathBuf {
    appdata_root().join("plugins")
}

/// 压缩状态存储目录：`<appdata>/compression`
pub(crate) fn compression_dir() -> std::path::PathBuf {
    appdata_root().join("compression")
}

/// 定时任务存储目录：`<appdata>/schedules`
pub(crate) fn schedules_dir() -> std::path::PathBuf {
    appdata_root().join("schedules")
}

/// embedding 向量缓存文件：`<appdata>/memory_embeddings.json`
pub(crate) fn embeddings_cache_path() -> std::path::PathBuf {
    appdata_root().join("memory_embeddings.json")
}

/// 永久记忆存储文件：`<appdata>/pinned_memories.json`
pub(crate) fn pinned_memories_path() -> std::path::PathBuf {
    appdata_root().join("pinned_memories.json")
}

/// 附件存储目录：`<appdata>/attachments`
///
/// ImageGenTool 把生成图片落盘到此目录；前端通过 read_attachment 命令读取。
pub(crate) fn attachments_dir() -> std::path::PathBuf {
    appdata_root().join("attachments")
}

/// ASR 转写记录存储目录：`<appdata>/asr`
///
/// AsrStore 在此目录下维护 records.json 索引与 transcripts/、audio/ 子目录。
pub(crate) fn asr_dir() -> std::path::PathBuf {
    appdata_root().join("asr")
}

/// P2P 信任库与配对数据存储目录：`<appdata>/p2p`
///
/// `TrustStore::load_or_create` 据此目录下的 `trust.json` 加载或生成身份。
/// 包含本机 Ed25519 私钥种子与已配对设备公钥表，切勿与其他设备共享。
pub(crate) fn p2p_dir() -> std::path::PathBuf {
    appdata_root().join("p2p")
}

/// P2P 信任库文件路径：`<appdata>/p2p/trust.json`
///
/// 供 `TrustStore::load_or_create` 直接使用。
pub(crate) fn p2p_trust_path() -> std::path::PathBuf {
    p2p_dir().join("trust.json")
}
