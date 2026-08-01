//! 文件 / 图片选择与读取命令。

/// 用户通过系统对话框选择的文件信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct PickedFile {
    pub path: String,
    pub name: String,
    pub size: u64,
}

/// 从 `FilePath` 提取 `(path_str, name, size)`，供 pick_file / pick_image 复用。
fn picked_file_info(path_str: String) -> PickedFile {
    let name = std::path::Path::new(&path_str)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let size = std::fs::metadata(&path_str).map(|m| m.len()).unwrap_or(0);
    PickedFile {
        path: path_str,
        name,
        size,
    }
}

/// 弹出系统文件选择对话框（文档/图片/所有文件）
#[tauri::command]
pub(crate) async fn pick_file(app: tauri::AppHandle) -> Result<Option<PickedFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .add_filter(
            "文档",
            &[
                "txt", "md", "pdf", "doc", "docx", "csv", "json", "rs", "py", "ts", "js",
            ],
        )
        .add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp"])
        .add_filter("所有文件", &["*"])
        .blocking_pick_file();
    Ok(path.map(|fp| picked_file_info(fp.to_string())))
}

/// 弹出系统图片选择对话框
#[tauri::command]
pub(crate) async fn pick_image(app: tauri::AppHandle) -> Result<Option<PickedFile>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"])
        .blocking_pick_file();
    Ok(path.map(|fp| picked_file_info(fp.to_string())))
}

/// 调起系统相机应用（桌面端简化为复用图片选择器作为相机替代）
#[tauri::command]
pub(crate) async fn capture_photo(app: tauri::AppHandle) -> Result<Option<PickedFile>, String> {
    pick_image(app).await
}

/// 读取文件文本内容（供 agent 使用），默认最多 512KB。
/// 截断处若落在多字节字符中间，回退到最后一个有效 UTF-8 边界。
#[tauri::command]
pub(crate) async fn read_file_text(path: String, max_bytes: Option<u64>) -> Result<String, String> {
    let max = max_bytes.unwrap_or(512 * 1024) as usize;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let truncated: &[u8] = if bytes.len() > max {
        &bytes[..max]
    } else {
        &bytes[..]
    };
    match std::str::from_utf8(truncated) {
        Ok(s) => Ok(s.to_string()),
        Err(e) => {
            let cut = e.valid_up_to();
            if cut == 0 {
                Err("文件内容不是有效的 UTF-8 文本".to_string())
            } else {
                Ok(std::str::from_utf8(&truncated[..cut])
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| "文件内容不是有效的 UTF-8 文本".to_string()))
            }
        }
    }
}

/// 调起系统目录选择对话框，返回所选目录的绝对路径。
///
/// 供前端设置技能/会话工作区时使用，避免用户手输路径出错。
#[tauri::command]
pub(crate) async fn pick_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .set_title("选择工作区目录")
        .blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}
