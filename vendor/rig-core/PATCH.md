# rig-core vendored patch

本目录是 rig-core 0.40.0 的完整源码副本（`cargo vendor` 等价物），
通过 workspace `[patch.crates-io]` 替换 crates.io 上的同名 crate。

## 为什么需要 vendor

rig-core 的 OpenAI 兼容 provider 在解析 Chat Completions 响应时，
只读取 `usage.prompt_tokens_details.cached_tokens`（OpenAI 标准字段），
而 DeepSeek 在 `usage` 顶层返回 `prompt_cache_hit_tokens` /
`prompt_cache_miss_tokens`，rig 会静默丢弃这两个字段，
导致程序无法区分缓存命中/未命中的输入 token，无法按 DeepSeek 计费规则算价。

## 补丁内容（相对上游 0.40.0）

文件：`src/providers/openai/completion/mod.rs`

1. `Usage` 结构体新增两个可选字段：
   - `prompt_cache_hit_tokens: Option<usize>`
   - `prompt_cache_miss_tokens: Option<usize>`
2. `Usage::new()` 初始化新字段为 `None`。
3. `GetTokenUsage for Usage::token_usage()`：
   - `prompt_cache_hit_tokens` 有值时覆盖 `cached_input_tokens`
   - `prompt_cache_miss_tokens` 有值时写入 `cache_creation_input_tokens`

其它 provider 不返回这两个字段（保持 `None`），行为与上游一致。

## 升级 rig-core 时的操作

1. 从 crates.io 拉取新版本源码，整体替换本目录。
2. 重新应用上述 3 处修改。
3. `cargo check` 验证。
