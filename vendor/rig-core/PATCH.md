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

### 1. DeepSeek 缓存 token 计费（原有）

文件：`src/providers/openai/completion/mod.rs`

1. `Usage` 结构体新增两个可选字段：
   - `prompt_cache_hit_tokens: Option<usize>`
   - `prompt_cache_miss_tokens: Option<usize>`
2. `Usage::new()` 初始化新字段为 `None`。
3. `GetTokenUsage for Usage::token_usage()`：
   - `prompt_cache_hit_tokens` 有值时覆盖 `cached_input_tokens`
   - `prompt_cache_miss_tokens` 有值时写入 `cache_creation_input_tokens`

其它 provider 不返回这两个字段（保持 `None`），行为与上游一致。

### 2. AI 工具参数支持 XML 输入（新增）

让全部 AI 工具在 JSON 之外多支持一种 XML 参数输入，缓解 LLM 生成 JSON 参数时的
转义问题（代码 / 正则 / 路径里的引号、反斜杠、换行容易写错）。

新增文件：`src/xml_tool_args.rs`（`pub(crate) mod xml_tool_args;`，在 `lib.rs` 注册）

XML 用 `<_KEY_>value</_KEY_>` 标签（下划线包裹，与常见标签区分），解析为 JSON 对象：
- 键名大小写不敏感（统一转小写，对齐 serde snake_case 字段名）
- 纯文本自动类型推断：bool / i64 / f64 / 字符串（首尾修剪）
- CDATA 内容原样保留（不推断类型、不修剪、不解码实体）
- 嵌套元素 → 对象；同一层 ≥2 个同名包裹元素（如 `<ITEM>`）→ 数组
- 普通文本解码标准 XML 实体（`&amp;` 等）

修改文件：`src/json_utils.rs`

`parse_tool_arguments` 先按 JSON 解析，失败时回退到
`xml_tool_args::parse_xml_tool_arguments`；两者都失败才返回原 JSON 错误。
该函数是非流式（`deserialize_maybe_stringified`）、OpenAI 兼容流式
（`openai_chat_completions_compatible`）与 Cohere 流式的共用咽喉，因此三条路径
同时获得 XML 支持。

## 升级 rig-core 时的操作

1. 从 crates.io 拉取新版本源码，整体替换本目录。
2. 重新应用上述 1、2 两处修改（含新增 `xml_tool_args.rs` 与 `lib.rs` 的模块注册）。
3. `cargo check` 验证。
