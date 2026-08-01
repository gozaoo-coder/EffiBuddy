use super::error::ClawHubError;

/// 把 ZIP 字节流解压到指定目录（同步，调用方应在 `spawn_blocking` 中调用）。
///
/// - 创建 `dest_dir`（若不存在）
/// - 跳过非文件条目（目录自动创建）
/// - 防御性拒绝绝对路径与 `..` 路径（zip-slip 攻击防护）
pub fn extract_zip_to(
    dest_dir: &std::path::Path,
    zip_bytes: &[u8],
) -> std::result::Result<(), ClawHubError> {
    use std::path::{Component, Path};
    std::fs::create_dir_all(dest_dir).map_err(ClawHubError::Io)?;
    // 规范化 dest_dir 用于后续比较（canonicalize 要求路径存在，create_dir_all 已确保）
    let dest_canon = dest_dir
        .canonicalize()
        .map_err(ClawHubError::Io)
        .or_else(|_| Ok::<_, ClawHubError>(dest_dir.to_path_buf()))?;
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| ClawHubError::Zip(e.to_string()))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| ClawHubError::Zip(e.to_string()))?;
        let entry_name = entry.name().to_string();
        // 逐组件检查：禁止 `..`、绝对路径前缀（Windows 盘符 / Unix 根）
        let entry_path = Path::new(&entry_name);
        for component in entry_path.components() {
            match component {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(ClawHubError::Zip(format!(
                        "zip entry 路径越权（含 `..` 或绝对路径）：{}",
                        entry_name
                    )));
                }
                _ => {}
            }
        }
        let out_path = dest_canon.join(entry_path);
        // 二次防御：canonicalize 父目录，确保最终路径仍在 dest_canon 之下
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(ClawHubError::Io)?;
            if let Ok(parent_canon) = parent.canonicalize() {
                if !parent_canon.starts_with(&dest_canon) {
                    return Err(ClawHubError::Zip(format!(
                        "zip entry 解析后路径越权：{}",
                        entry_name
                    )));
                }
            }
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(ClawHubError::Io)?;
            continue;
        }
        let mut out_file = std::fs::File::create(&out_path).map_err(ClawHubError::Io)?;
        std::io::copy(&mut entry, &mut out_file).map_err(ClawHubError::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extract_zip_rejects_path_traversal() {
        // 构造一个恶意 zip：含 `../evil.txt` 条目
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions = Default::default();
            zip.start_file("../evil.txt", opts).unwrap();
            zip.write_all(b"pwned").unwrap();
            zip.finish().unwrap();
        }
        let tmp = std::env::temp_dir().join(format!("effisuite-zip-test-{}", uuid::Uuid::new_v4()));
        let result = extract_zip_to(&tmp, &buf.into_inner());
        assert!(result.is_err(), "extract_zip_to 应拒绝路径越权条目");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
