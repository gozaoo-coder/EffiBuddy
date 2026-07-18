//! display_image 工具：让 LLM 把已有图片发送到聊天框显示给用户
//!
//! 与 [`ImageGenTool`] 互补：
//! - `image_gen`：调用模型**生成**新图片
//! - `display_image`：把**已有**图片（本地文件或网络 URL）推送到聊天框
//!
//! 典型场景：
//! - LLM 用 shell/read_file 浏览工作区后，发现一张图片，想直接展示给用户
//! - LLM 用 web_fetch 抓取网页时拿到图片 URL，想嵌入聊天
//! - LLM 处理用户拖入的图片（attachments 目录内），想回显
//!
//! ## 工作流程
//! 1. LLM 传入 `local_path` 或 `url`（二选一）
//! 2. 工具把图片复制 / 下载到 `attachments_dir`，统一用 UUID 命名避免冲突
//! 3. 返回 `ImageGenOutput` 格式（与 image_gen 工具一致），便于复用
//!    `parse_image_gen_output` 与 `agent-attachment` 事件链路
//! 4. Tauri 命令层流式处理 ToolResult 时，识别 `display_image` 工具，
//!    emit `agent-attachment` 事件，前端立即渲染
//!
//! ## 安全性
//! - 本地路径：信任本地 agent 环境，不做沙箱限制（与 read_file 一致）
//! - URL：仅允许 http/https，禁止 file/ftp 等
//! - 大小限制：单张 20 MiB，防止下载超大文件撑爆磁盘

use std::path::PathBuf;

use rig_core::tool::Tool;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use super::resolve_path;

/// 单张图片最大字节数（20 MiB）
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// 工具参数
///
/// `local_path` 与 `url` 二选一：同时提供时优先 `local_path`。
#[derive(Deserialize)]
pub struct DisplayImageArgs {
    /// 本地图片路径（绝对或相对工作区）。支持 png/jpg/jpeg/gif/webp/bmp/svg。
    #[serde(default)]
    pub local_path: Option<String>,
    /// 网络图片 URL（http/https）
    #[serde(default)]
    pub url: Option<String>,
    /// 可选显示名（不含扩展名）。留空时用源文件名或"图片"
    #[serde(default)]
    pub name: Option<String>,
}

/// 工具错误
#[derive(Debug, thiserror::Error)]
#[error("display_image error: {0}")]
pub struct DisplayImageError(String);

/// 工具输出：与 ImageGenOutput 同构，便于复用前端事件链路
#[derive(Debug, serde::Serialize)]
pub struct DisplayImageOutput {
    /// 附件 id（同时作为文件名前缀）
    pub id: String,
    /// 相对 attachments 目录的文件名
    pub path: String,
    /// 显示用文件名
    pub name: String,
    /// 来源：local（本地复制）/ url（网络下载）
    pub source: String,
    /// 耗时毫秒
    pub elapsed_ms: u64,
}

/// 图片展示工具
///
/// `cwd` 为可选工作区：设置后相对 local_path 以此为基准。
/// `attachments_dir` 为图片落盘目录（与 ImageGenTool 共享）。
pub struct DisplayImageTool {
    cwd: Option<PathBuf>,
    attachments_dir: PathBuf,
}

impl DisplayImageTool {
    pub fn new(attachments_dir: PathBuf) -> Self {
        Self {
            cwd: None,
            attachments_dir,
        }
    }

    /// 指定工作区目录，相对 local_path 以此为基准
    pub fn with_cwd(cwd: PathBuf, attachments_dir: PathBuf) -> Self {
        Self {
            cwd: Some(cwd),
            attachments_dir,
        }
    }
}

/// 支持的图片扩展名（小写匹配）
const SUPPORTED_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

/// 判断路径扩展名是否为支持的图片格式
fn is_supported_ext(filename: &str) -> bool {
    filename
        .rsplit('.')
        .next()
        .map(|e| SUPPORTED_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// 从文件名提取扩展名（含点，小写）。无扩展名时返回 ".png" 作为默认
fn ext_of(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.ends_with(".png") {
        ".png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        ".jpg"
    } else if lower.ends_with(".gif") {
        ".gif"
    } else if lower.ends_with(".webp") {
        ".webp"
    } else if lower.ends_with(".bmp") {
        ".bmp"
    } else if lower.ends_with(".svg") {
        ".svg"
    } else {
        ".png"
    }
}

impl Tool for DisplayImageTool {
    const NAME: &'static str = "display_image";

    type Error = DisplayImageError;
    type Args = DisplayImageArgs;
    type Output = DisplayImageOutput;

    fn description(&self) -> String {
        "把已有图片发送到聊天框显示给用户。支持本地文件路径或网络 URL。\
         用于展示工作区中的图片、回显用户上传的图片、或嵌入网络图片。\
         与 image_gen（生成新图片）互补：本工具只搬运已有图片，不调用生成模型。"
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "local_path": {
                    "type": "string",
                    "description": "本地图片路径（绝对或相对工作区）。支持 png/jpg/jpeg/gif/webp/bmp/svg。与 url 二选一，同时提供时优先使用。"
                },
                "url": {
                    "type": "string",
                    "description": "网络图片 URL（仅 http/https）。与 local_path 二选一。"
                },
                "name": {
                    "type": "string",
                    "description": "可选显示名（不含扩展名）。留空时用源文件名或'图片'。"
                }
            }
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let started = std::time::Instant::now();

        // 确保 attachments 目录存在
        tokio::fs::create_dir_all(&self.attachments_dir)
            .await
            .map_err(|e| DisplayImageError(format!("创建 attachments 目录失败: {e}")))?;

        // 生成唯一 id 与目标路径
        let id = uuid::Uuid::new_v4().to_string();
        let (bytes, ext, display_name, source) = if let Some(lp) = args.local_path.as_deref() {
            // 本地路径分支
            let resolved = resolve_path(lp, self.cwd.as_deref());
            let filename = resolved
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("image.png");
            if !is_supported_ext(filename) {
                return Err(DisplayImageError(format!(
                    "不支持的图片格式：{filename}（支持 png/jpg/jpeg/gif/webp/bmp/svg）"
                )));
            }
            let ext = ext_of(filename);
            let bytes = tokio::fs::read(&resolved)
                .await
                .map_err(|e| DisplayImageError(format!("读取本地图片失败 [{}]: {e}", resolved.display())))?;
            let display_name = args
                .name
                .clone()
                .or_else(|| {
                    resolved
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "图片".to_string());
            (bytes, ext, display_name, "local")
        } else if let Some(url) = args.url.as_deref() {
            // URL 分支
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(DisplayImageError(
                    "仅支持 http/https URL，禁止 file/ftp 等协议".into(),
                ));
            }
            // 从 URL 推断扩展名
            let url_path = url.split('?').next().unwrap_or(url);
            let ext = ext_of(url_path);
            let bytes = download_image(url)
                .await
                .map_err(|e| DisplayImageError(format!("下载图片失败 [{url}]: {e}")))?;
            let display_name = args
                .name
                .clone()
                .or_else(|| {
                    url_path
                        .rsplit('/')
                        .next()
                        .and_then(|s| s.rsplit_once('.').map(|(stem, _)| stem.to_string()))
                })
                .unwrap_or_else(|| "图片".to_string());
            (bytes, ext, display_name, "url")
        } else {
            return Err(DisplayImageError(
                "必须提供 local_path 或 url 之一".into(),
            ));
        };

        // 大小限制
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(DisplayImageError(format!(
                "图片过大（{} 字节），超过单张上限 {} 字节",
                bytes.len(),
                MAX_IMAGE_BYTES
            )));
        }

        // 落盘到 attachments 目录
        let filename = format!("disp_{id}{ext}");
        let dest = self.attachments_dir.join(&filename);
        let mut file = tokio::fs::File::create(&dest)
            .await
            .map_err(|e| DisplayImageError(format!("创建目标文件失败 [{}]: {e}", dest.display())))?;
        file.write_all(&bytes)
            .await
            .map_err(|e| DisplayImageError(format!("写入图片失败: {e}")))?;
        file.flush()
            .await
            .map_err(|e| DisplayImageError(format!("flush 失败: {e}")))?;
        drop(file);

        Ok(DisplayImageOutput {
            id,
            path: filename,
            name: format!("{display_name}{ext}"),
            source: source.to_string(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// 下载网络图片
///
/// 用 reqwest（已在 workspace 依赖中）发起 GET 请求，限制响应大小。
async fn download_image(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    // 限制响应体大小，防止恶意大文件
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应体失败: {e}"))?;
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_detection() {
        assert_eq!(ext_of("photo.png"), ".png");
        assert_eq!(ext_of("photo.PNG"), ".png");
        assert_eq!(ext_of("photo.jpg"), ".jpg");
        assert_eq!(ext_of("photo.JPEG"), ".jpg");
        assert_eq!(ext_of("photo.jpeg"), ".jpg");
        assert_eq!(ext_of("photo.gif"), ".gif");
        assert_eq!(ext_of("photo.webp"), ".webp");
        assert_eq!(ext_of("photo.svg"), ".svg");
        assert_eq!(ext_of("noext"), ".png"); // 默认
        assert_eq!(ext_of("photo.txt"), ".png"); // 不识别的扩展名用默认
    }

    #[test]
    fn supported_ext_check() {
        assert!(is_supported_ext("a.png"));
        assert!(is_supported_ext("a.JPG"));
        assert!(is_supported_ext("a.webp"));
        assert!(!is_supported_ext("a.txt"));
        assert!(!is_supported_ext("noext"));
    }

    #[tokio::test]
    async fn display_local_image_roundtrip() {
        let dir = std::env::temp_dir().join(format!("effisuite-display-test-{}", uuid::Uuid::new_v4()));
        let src_dir = dir.join("src");
        let att_dir = dir.join("attachments");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&att_dir).unwrap();

        // 准备源图片（最小 PNG：8 字节签名 + IHDR chunk）
        let png_bytes = b"\x89PNG\r\n\x1a\nfake_png_data_for_test";
        let src_path = src_dir.join("test.png");
        std::fs::write(&src_path, png_bytes).unwrap();

        let tool = DisplayImageTool::with_cwd(src_dir.clone(), att_dir.clone());
        let result = tool
            .call(DisplayImageArgs {
                local_path: Some("test.png".to_string()),
                url: None,
                name: Some("测试图片".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(result.source, "local");
        assert!(result.path.starts_with("disp_"));
        assert!(result.path.ends_with(".png"));
        assert_eq!(result.name, "测试图片.png");

        // 验证落盘内容
        let written = std::fs::read(att_dir.join(&result.path)).unwrap();
        assert_eq!(written, png_bytes);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn display_rejects_unsupported_ext() {
        let dir = std::env::temp_dir().join(format!("effisuite-display-ext-{}", uuid::Uuid::new_v4()));
        let att_dir = dir.join("attachments");
        std::fs::create_dir_all(&att_dir).unwrap();
        let tool = DisplayImageTool::with_cwd(dir.clone(), att_dir.clone());

        // 准备一个 txt 文件
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let result = tool
            .call(DisplayImageArgs {
                local_path: Some("a.txt".to_string()),
                url: None,
                name: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不支持的图片格式"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn display_rejects_missing_source() {
        let dir = std::env::temp_dir().join(format!("effisuite-display-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = DisplayImageTool::new(dir.clone());

        let result = tool
            .call(DisplayImageArgs {
                local_path: None,
                url: None,
                name: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("必须提供"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn display_rejects_non_http_url() {
        let dir = std::env::temp_dir().join(format!("effisuite-display-url-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = DisplayImageTool::new(dir.clone());

        let result = tool
            .call(DisplayImageArgs {
                local_path: None,
                url: Some("file:///etc/passwd".to_string()),
                name: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("仅支持 http/https"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn display_rejects_oversized_image() {
        let dir = std::env::temp_dir().join(format!("effisuite-display-big-{}", uuid::Uuid::new_v4()));
        let att_dir = dir.join("attachments");
        std::fs::create_dir_all(&att_dir).unwrap();
        let tool = DisplayImageTool::with_cwd(dir.clone(), att_dir.clone());

        // 准备一个超过 20 MiB 的"图片"
        let big = vec![0u8; MAX_IMAGE_BYTES + 1];
        std::fs::write(dir.join("big.png"), &big).unwrap();
        let result = tool
            .call(DisplayImageArgs {
                local_path: Some("big.png".to_string()),
                url: None,
                name: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("图片过大"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn display_local_path_priority_over_url() {
        // 同时提供 local_path 与 url 时，优先 local_path
        let dir = std::env::temp_dir().join(format!("effisuite-display-prio-{}", uuid::Uuid::new_v4()));
        let att_dir = dir.join("attachments");
        std::fs::create_dir_all(&att_dir).unwrap();
        let tool = DisplayImageTool::with_cwd(dir.clone(), att_dir.clone());

        std::fs::write(dir.join("a.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let result = tool
            .call(DisplayImageArgs {
                local_path: Some("a.png".to_string()),
                url: Some("https://example.com/x.png".to_string()),
                name: None,
            })
            .await
            .unwrap();
        assert_eq!(result.source, "local");

        std::fs::remove_dir_all(&dir).ok();
    }
}
