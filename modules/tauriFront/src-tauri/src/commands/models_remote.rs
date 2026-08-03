//! 远程模型拉取：调用 OpenAI 兼容 `/models` 接口。
//! 独立成模块，避免 `models.rs`（Provider 预设 / 可使用模型管理 / 图像生成）
//! 继续膨胀为上帝文件。本模块职责：
//! - 智能拼接模型列表 / 详情 URL（容忍各种 base_url 写法）
//! - 多态解析响应（`{data:[...]}` / `{models:[...]}` / 裸数组 / 字段兼容）
//! - 无 API Key 时放行（本地 Ollama 等无鉴权服务不携带 Authorization）
//! - 清晰错误信息（HTTP 状态码 + 服务端 error 字段 + 可读提示）
//!
//! 命令在 `commands/mod.rs` 统一 re-export，经 `lib.rs` 的 `invoke_handler!` 注册。

use serde_json::Value;

use crate::commands::chat::truncate_str;


/// 远程模型条目（OpenAI `/v1/models` 响应中的元素）
///
/// 字段对齐 OpenAI 官方 API；id 为模型标识（如 gpt-4o-mini），
/// owned_by 为归属（openai / organization-owner / ...）。
/// 其余 provider 字段（permission / digest / size 等）对前端无用，直接丢弃。
#[derive(Debug, serde::Serialize)]
pub(crate) struct RemoteModelInfo {
    id: String,
    object: String,
    owned_by: String,
    /// 模型创建时间（Unix 秒），部分 provider 不返回，留 None
    created: Option<u64>,
}

// =========================================================
// URL 智能拼接
// =========================================================

/// 生成模型列表接口的候选 URL 列表（按优先级排列）。
///
/// 容忍用户填写的各种 base_url 形态：
/// - 尾部多余 `/` 与空白：自动去除
/// - 已含 `/models`：不再重复拼接
/// - 以 `/v1` 结尾：直接拼 `/models`（OpenAI 标准）
/// - 无 `/v1`（如 `https://api.deepseek.com`、`http://localhost:11434`）：
///   同时尝试 `/v1/models` 与 `/models`，由调用方按 404 回退
fn models_url_candidates(base_url: &str) -> Vec<String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Vec::new();
    }
    let lower = base.to_ascii_lowercase();
    if lower.ends_with("/models") {
        return vec![base.to_string()];
    }
    if lower.ends_with("/v1") {
        return vec![format!("{base}/models")];
    }
    // OpenAI 兼容网关一般挂在 /v1 下；部分服务（DeepSeek 根路径、自建网关）直接 /models
    vec![format!("{base}/v1/models"), format!("{base}/models")]
}

/// URL 路径段编码（model id 含 `:` `/` `@` 等需转义，按 UTF-8 字节编码）
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// =========================================================
// 响应多态解析
// =========================================================

/// 从错误响应体提取可读错误信息（OpenAI 风格 `{"error":{"message":...}}` 或纯字符串）
fn extract_error_message(v: &Value) -> Option<String> {
    let err = v.get("error")?;
    if let Some(msg) = err.get("message").and_then(Value::as_str) {
        return Some(msg.to_string());
    }
    if let Some(msg) = err.as_str() {
        return Some(msg.to_string());
    }
    if let Some(msg) = err.get("msg").and_then(Value::as_str) {
        return Some(msg.to_string());
    }
    None
}

/// 把响应体解析为模型列表（多态）：
/// - OpenAI 标准：`{ "object": "list", "data": [ {id, object, owned_by, created} ] }`
/// - Ollama / LM Studio 变体：`{ "models": [ {name|model, ...} ] }`
/// - 部分网关：裸数组 `[ {id, ...} ]`
///
/// 字段兼容：id 缺省时回退到 `name` / `model`。
/// 容器存在但为空数组时返回空列表（合法），找不到任何容器结构才报错。
fn parse_models_response(body: &[u8]) -> Result<Vec<RemoteModelInfo>, String> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| format!("响应不是合法 JSON: {e}（该服务可能不是 OpenAI 兼容接口）"))?;

    let arr: Option<&Vec<Value>> = v
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| v.get("models").and_then(Value::as_array))
        .or_else(|| v.as_array());

    let arr = match arr {
        Some(a) => a,
        None => {
            if let Some(msg) = extract_error_message(&v) {
                return Err(format!("服务端返回错误: {msg}"));
            }
            return Err("响应中未找到模型列表（期望 data 数组 / models 数组 / 裸数组）".into());
        }
    };

    let mut list = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| obj.get("name").and_then(Value::as_str))
            .or_else(|| obj.get("model").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }
        list.push(RemoteModelInfo {
            id,
            object: obj
                .get("object")
                .and_then(Value::as_str)
                .unwrap_or("model")
                .to_string(),
            owned_by: obj
                .get("owned_by")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            created: obj.get("created").and_then(Value::as_u64),
        });
    }
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}

/// 解析单个模型详情（兼容 `{ data: {...} }` 与裸对象）
fn parse_model_detail_response(body: &[u8]) -> Result<RemoteModelInfo, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("解析响应失败: {e}"))?;
    if let Some(msg) = extract_error_message(&v) {
        return Err(format!("服务端返回错误: {msg}"));
    }
    let obj = v.get("data").filter(|d| d.is_object()).unwrap_or(&v);
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| obj.get("model").and_then(Value::as_str))
        .or_else(|| obj.get("name").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    if id.is_empty() {
        return Err("响应中未找到模型 id".into());
    }
    Ok(RemoteModelInfo {
        id,
        object: obj
            .get("object")
            .and_then(Value::as_str)
            .unwrap_or("model")
            .to_string(),
        owned_by: obj
            .get("owned_by")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created: obj.get("created").and_then(Value::as_u64),
    })
}

/// 构造可读的 HTTP 错误信息：状态码 + 服务端 error 字段/截断 body + 排查提示
fn format_http_error(status: u16, body: &str) -> String {
    let hint = match status {
        401 | 403 => "，请检查 API Key 是否有效（若该服务无需鉴权可清空 Key 重试）",
        404 => "，接口路径不存在（已自动尝试 /v1/models 与 /models）",
        408 | 429 => "，请求超时或被限流",
        500..=599 => "，服务端错误",
        _ => "",
    };
    let detail = extract_error_message(
        &serde_json::from_str::<Value>(body).unwrap_or_else(|_| Value::Null),
    )
    .unwrap_or_else(|| truncate_str(body.trim(), 200));
    format!("HTTP {status}{hint}: {detail}")
}

// =========================================================
// 命令
// =========================================================

/// 列出 API 可用模型（OpenAI 兼容 `GET {base_url}/models`）。
///
/// - `base_url`：API 基地址（如 `https://api.openai.com/v1`）
/// - `api_key`：Bearer token；为空时（本地 Ollama 等无鉴权服务）不携带 Authorization
#[tauri::command]
pub(crate) async fn list_remote_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<RemoteModelInfo>, String> {
    let candidates = models_url_candidates(&base_url);
    if candidates.is_empty() {
        return Err("Base URL 为空".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
    let key = api_key.trim();

    // last_err: (是否仅为路径 404, 消息)。非 404 错误更信息量，优先保留。
    let mut last_err: Option<(bool, String)> = None;
    let record_err = |is_404: bool, msg: String, slot: &mut Option<(bool, String)>| {
        match slot {
            None => *slot = Some((is_404, msg)),
            Some((prev_is_404, _)) => {
                if !is_404 || *prev_is_404 {
                    *slot = Some((is_404, msg));
                }
            }
        }
    };

    for url in &candidates {
        let mut req = client.get(url.as_str());
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                record_err(false, format!("请求 {url} 失败: {e}"), &mut last_err);
                continue;
            }
        };
        let status = resp.status();
        // 路径不存在：尝试下一个候选；其余非成功状态为真实错误，直接返回
        if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            record_err(true, format!("{url} 返回 HTTP {status}"), &mut last_err);
            continue;
        }
        let body_bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                record_err(false, format!("读取响应体失败: {e}"), &mut last_err);
                continue;
            }
        };
        if !status.is_success() {
            let body_str = String::from_utf8_lossy(&body_bytes).to_string();
            return Err(format_http_error(status.as_u16(), &body_str));
        }
        match parse_models_response(&body_bytes) {
            Ok(list) => return Ok(list),
            Err(e) => {
                record_err(false, format!("{url} 解析失败: {e}"), &mut last_err);
            }
        }
    }
    Err(last_err
        .map(|(_, m)| m)
        .unwrap_or_else(|| "无法连接模型 API，请检查网络或 Base URL".into()))
}

/// 检索单个模型详情（OpenAI 兼容 `GET {base_url}/models/{model}`）。
#[tauri::command]
pub(crate) async fn get_remote_model(
    base_url: String,
    api_key: String,
    model: String,
) -> Result<RemoteModelInfo, String> {
    let list_url = models_url_candidates(&base_url)
        .into_iter()
        .next()
        .unwrap_or_default();
    if list_url.is_empty() {
        return Err("Base URL 为空".into());
    }
    let encoded = urlencoding_encode(&model);
    let url = if list_url.to_ascii_lowercase().ends_with("/models") {
        format!("{list_url}/{encoded}")
    } else {
        format!("{}/models/{encoded}", list_url.trim_end_matches('/'))
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
    let key = api_key.trim();
    let mut req = client.get(url.as_str());
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 {url} 失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format_http_error(status.as_u16(), &body));
    }
    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应体失败: {e}"))?;
    parse_model_detail_response(&body_bytes)
}

// =========================================================
// 单元测试
// =========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_candidates_various_shapes() {
        // OpenAI 标准（带 /v1）：单一候选
        assert_eq!(
            models_url_candidates("https://api.openai.com/v1"),
            vec!["https://api.openai.com/v1/models"]
        );
        // 尾部斜杠与空白
        assert_eq!(
            models_url_candidates("  https://api.openai.com/v1/  "),
            vec!["https://api.openai.com/v1/models"]
        );
        // 无 /v1：DeepSeek / 自建网关 / 本地 Ollama → 两个候选
        assert_eq!(
            models_url_candidates("https://api.deepseek.com"),
            vec![
                "https://api.deepseek.com/v1/models",
                "https://api.deepseek.com/models"
            ]
        );
        assert_eq!(
            models_url_candidates("http://localhost:11434"),
            vec![
                "http://localhost:11434/v1/models",
                "http://localhost:11434/models"
            ]
        );
        // 已含 /models：不重复拼接
        assert_eq!(
            models_url_candidates("https://host:8080/v1/models"),
            vec!["https://host:8080/v1/models"]
        );
        // 空输入
        assert!(models_url_candidates("").is_empty());
        assert!(models_url_candidates("  ").is_empty());
    }

    #[test]
    fn parse_openai_standard() {
        let body = br#"{
            "object": "list",
            "data": [
                {"id": "gpt-4o-mini", "object": "model", "created": 1700000000, "owned_by": "openai"},
                {"id": "gpt-4o", "object": "model", "created": 1700000001, "owned_by": "system"}
            ]
        }"#;
        let list = parse_models_response(body).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "gpt-4o"); // 已按 id 排序
        assert_eq!(list[0].owned_by, "system");
        assert_eq!(list[1].id, "gpt-4o-mini");
    }

    #[test]
    fn parse_bare_array() {
        let body = br#"[{"id": "m1"}, {"id": "m2", "owned_by": "me"}]"#;
        let list = parse_models_response(body).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "m1");
        assert_eq!(list[0].object, "model"); // 缺省 object
        assert_eq!(list[1].owned_by, "me");
    }

    #[test]
    fn parse_ollama_like_models_field() {
        // Ollama /api/tags 风格：models 数组，字段是 name/model 而非 id
        let body = br#"{
            "models": [
                {"name": "llama3:8b", "model": "llama3:8b", "size": 1234, "modified_at": "2024-01-01"},
                {"name": "qwen2.5:7b", "model": "qwen2.5:7b", "size": 5678}
            ]
        }"#;
        let list = parse_models_response(body).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "llama3:8b");
        assert_eq!(list[1].id, "qwen2.5:7b");
    }

    #[test]
    fn parse_empty_data_is_ok() {
        let list = parse_models_response(br#"{"object":"list","data":[]}"#).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn parse_error_object_returns_message() {
        let err = parse_models_response(
            br#"{"error": {"message": "Invalid API key", "type": "invalid_request_error"}}"#,
        )
        .unwrap_err();
        assert!(err.contains("Invalid API key"), "err={err}");
    }

    #[test]
    fn parse_unrecognized_shape() {
        let err = parse_models_response(br#"{"foo": 1}"#).unwrap_err();
        assert!(err.contains("未找到模型列表"), "err={err}");
        let err = parse_models_response(br#"not json at all"#).unwrap_err();
        assert!(err.contains("JSON"), "err={err}");
    }

    #[test]
    fn parse_model_detail_wrappers() {
        // 裸对象
        let m = parse_model_detail_response(br#"{"id":"gpt-4o","object":"model"}"#).unwrap();
        assert_eq!(m.id, "gpt-4o");
        // data 包裹
        let m =
            parse_model_detail_response(br#"{"data":{"id":"deepseek-chat","owned_by":"deepseek"}}"#)
                .unwrap();
        assert_eq!(m.id, "deepseek-chat");
        assert_eq!(m.owned_by, "deepseek");
        // 错误对象
        let err = parse_model_detail_response(br#"{"error":{"message":"not found"}}"#).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn urlencoding_handles_special_chars() {
        assert_eq!(urlencoding_encode("gpt-4o"), "gpt-4o");
        assert_eq!(urlencoding_encode("a/b:c"), "a%2Fb%3Ac");
        assert_eq!(urlencoding_encode("中文"), "%E4%B8%AD%E6%96%87");
    }

    #[test]
    fn http_error_message_includes_hints() {
        let msg = format_http_error(401, r#"{"error":{"message":"bad key"}}"#);
        assert!(msg.contains("401"));
        assert!(msg.contains("API Key"));
        assert!(msg.contains("bad key"));
        let msg = format_http_error(404, "page not found");
        assert!(msg.contains("404"));
        assert!(msg.contains("page not found"));
    }
}
