//! 搜索根解析与文件过滤

use std::path::{Path, PathBuf};

use super::constants::CODE_EXTS;
use super::SearchCodebaseError;

/// 解析搜索根目录列表
///
/// - `None` 或空列表 → 使用 cwd（或回退到 "."）
/// - `Some(dirs)` → 对每个路径用 `resolve_path` 解析为绝对路径
pub(super) fn resolve_roots(
    target_directories: &Option<Vec<String>>,
    cwd: Option<&Path>,
) -> Result<Vec<PathBuf>, SearchCodebaseError> {
    let fallback = || -> PathBuf {
        cwd.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    };

    match target_directories {
        None => {
            let root = fallback();
            validate_root(&root)?;
            Ok(vec![root])
        }
        Some(dirs) if dirs.is_empty() => {
            let root = fallback();
            validate_root(&root)?;
            Ok(vec![root])
        }
        Some(dirs) => {
            let mut roots: Vec<PathBuf> = Vec::with_capacity(dirs.len());
            for d in dirs {
                let trimmed = d.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let p = resolve_path(trimmed, cwd);
                validate_root(&p)?;
                roots.push(p);
            }
            Ok(roots)
        }
    }
}

/// 校验搜索根目录是否存在且为目录
fn validate_root(p: &Path) -> Result<(), SearchCodebaseError> {
    let meta = std::fs::metadata(p).map_err(|e| {
        SearchCodebaseError(format!("访问搜索根目录失败 [{}]: {e}", p.display()))
    })?;
    if !meta.is_dir() {
        return Err(SearchCodebaseError(format!(
            "搜索根不是目录 [{}]",
            p.display()
        )));
    }
    Ok(())
}

/// 判断是否为代码文件扩展名
#[inline]
pub(super) fn is_code_file(name: &str) -> bool {
    let ext = match name.rsplit('.').next() {
        Some(e) => e,
        None => return false,
    };
    CODE_EXTS.contains(&ext)
}

/// 判断是否为二进制文件：探测前 8KB 是否含 NUL 字节
#[inline]
pub(super) fn is_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(8192)];
    probe.contains(&0)
}

/// 解析路径：绝对路径或 cwd 为 None 时按原样返回；相对路径 join 到 cwd。
///
/// 转发到 `tools::resolve_path`（walker 位于 `tools::search_codebase::walker`，
/// 故 `super::super` 指向 `tools`）。
#[inline]
fn resolve_path(path: &str, cwd: Option<&Path>) -> PathBuf {
    super::super::resolve_path(path, cwd)
}
