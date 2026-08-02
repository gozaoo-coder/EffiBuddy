# EffiSuite

> 个人效率助手 · 本地优先的 AI Agent 桌面应用

EffiSuite 是一个基于 **Tauri 2 + Rust + Vue 3** 的桌面端 AI 助手。它把「LLM 对话」、「上下文记忆」、「工具调用」、「语音转写」、「多设备协作」等能力整合到一个应用中，让 AI 不仅能聊天，还能真正帮你读写文件、跑命令、搜代码、管理任务、跨设备协作。

```
┌─────────────────────────────────────────────────────────┐
│  IconRail │ History/池/模型 │  多页签聊天区（TabContent）   │
│   🗨 聊天  │   会话历史      │  流式回复 / 工具调用 / 子Agent  │
│   📡 交流池│   AgentPool    │  ASR 录入 / 转写 / 历史       │
│   ⚙ 模型  │   模型配置      │  P2P / 技能 / 插件 / 定时任务  │
│   🤖 技能  │               │                             │
└─────────────────────────────────────────────────────────┘
```

## ✨ 特性

- **AI 对话与工具调用** — 基于 [rig](https://crates.io/crates/rig-core) 驱动任意 OpenAI 兼容接口（OpenAI / DeepSeek / 本地 Ollama / 自建网关…），支持流式输出、工具调用（Function Calling）、思考过程展示；无网络时可用内置 `MockAgent` 离线回显。
- **35+ 工具集** — 文件读写/精确编辑/删除、Shell 命令、Web 搜索与抓取、代码库检索（glob / grep / 关键词加权搜索）、模型管理、图像/视频生成、子 Agent 委派、待办清单、定时任务等，全部在对话中即调即用。
- **分层上下文系统** — 每轮对话自动注入：永久记忆 → RAG 历史记忆（BM25 + 语义向量混合检索）→ 可用技能 → 待办清单（todoTree）→ Agent 交流池 → 当前对话。长会话由 LLM 压缩归并，控制 token 预算。
- **技能（Skill）体系** — 内置技能开箱即用，也可从 **ClawHub** 远程技能市场一键安装；技能以独立 system preamble 注入，能力即插即用。
- **语音转写（ASR）** — 支持火山引擎流式实时转写与通义千问文件转写，自动生成摘要、可检索历史记录。
- **P2P 多设备协作** — 局域网内可信设备发现 / 配对 / 端到端加密传输（X25519 + AES-256-GCM），支持镜像同步与「远端任务派发」（本机 AI 直接驱动另一台设备上的 AI）。
- **Agent 交流池** — 多个长任务 Agent 可在公共会话池登记状态、互相 @ 询问，避免重复劳动与文件操作冲突。
- **定时任务** — cron 表达式驱动的自动化调度面板。
- **多页签 + 会话管理** — 多对话并行、会话历史、文件夹归组、自动命名。
- **模型精细化管理** — 多 Provider 预设、服务角色映射（聊天 / 会话命名 / 压缩 / ASR 摘要各用各的模型）、图像模型独立配置。

## 🧱 架构总览

Cargo workspace，四个成员 + 一个 vendored patch：

```
EffiSuite
├── modules/core            effisuite-core   共享数据结构 / 配置 / 事件总线 / 持久化
│                                            · 记忆索引（BM25+向量混合 RAG）
│                                            · ClawHub 客户端 / 技能索引
│                                            · ASR 存储 / 压缩 / 定时任务存储
├── modules/agent           effisuite-agent  AI Agent 运行时
│                                            · 基于 rig 的对话后端（Mock / OpenAI 兼容）
│                                            · 35+ 工具实现（文件 / shell / web / 子Agent…）
│                                            · 上下文拼装（context.rs）/ 会话压缩
│                                            · ASR 服务（火山 / 通义）/ shell session / todo
├── modules/p2pConnection   effisuite-p2p    局域网 P2P：发现 / 配对 / 加密传输 / 镜像同步
├── modules/tauriFront      (前端 + 桌面壳)   Vue 3 + TS + Vite 前端
│   └── src-tauri                             Tauri 2 壳 + IPC 命令层 / 事件 / 调度器
└── vendor/rig-core                            vendored rig-core 0.40（DeepSeek 缓存 token 计费补丁）
```

模块间通过 Rust trait 抽象解耦（如 `ChatAgent`、`DiscoveryService`），业务层面向接口编程，后端可低成本替换。

## 🚀 快速开始

### 环境要求

| 依赖 | 版本 |
|---|---|
| [Rust](https://www.rust-lang.org/) | stable（见 `Cargo.toml` 的 `rust-version`） |
| [Node.js](https://nodejs.org/) | ≥ 20（前端构建） |
| [Tauri CLI](https://v2.tauri.app/) | 2.x（`cargo install tauri-cli` 或 `npm i -g @tauri-apps/cli`） |

> 首次构建 Tauri 应用时，Windows 需要 [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/)（Win10/11 一般已内置）。

### 安装依赖

```bash
cd modules/tauriFront
npm install
```

### 开发模式（热更新）

```bash
# 回到仓库根目录
cargo tauri dev
```

会自动启动 Vite 开发服务器（`http://localhost:5173`）并打开应用窗口。

### 生产构建

```bash
cargo tauri build
```

产物输出到 `modules/tauriFront/src-tauri/target/release/bundle/`。

### 仅跑 Rust 测试

```bash
cargo test
```

## 🧭 功能导览

### 首次使用：配置模型

1. 点击左侧栏 **模型配置**，在「AI 服务商」选择一个预设（如 OpenAI / DeepSeek / 自定义）。
2. 填入 `base_url` / `api_key` / `model_name`，保存。
3. 底部「聊天模型」角色选择刚保存的模型 → 即可开始对话。

不配置任何模型时，应用以 `MockAgent` 离线模式运行，方便先看界面。

### 对话与工具

在聊天输入框直接下达自然语言指令即可，AI 会自动决定是否调用工具（工具调用过程会以卡片/气泡形式实时展示）。常用能力示例：

- 「帮我看看 `src/main.rs` 并总结逻辑」→ `read_file`
- 「把第 12 行改成 `let x = 1`」→ `read_file` + `edit_file`
- 「搜索代码里处理登录的部分」→ `search_codebase`
- 「给当前项目生成一张 logo」→ `image_gen`
- 「让子 agent 审查这段代码的安全性」→ `sub_agent`
- 「把这个任务派发到我的手机上」→ `dispatch_remote_task`（需先完成 P2P 配对）

### 上下文与记忆

- **永久记忆**：输入框上方可置顶长期偏好/事实，每轮自动注入。
- **历史记忆**：跨会话 RAG 检索，命中相关历史片段。
- **技能**：内置技能直接启用；ClawHub 面板搜索并一键安装第三方技能。
- **待办清单**：AI 通过 `todo_write` 维护多步任务清单，随上下文注入。
- **Agent 交流池**：长任务在此登记状态，多会话并行协作。

### 语音转写（ASR）

左侧栏打开 **ASR 录入** 可实时语音转文字；**ASR 转写** 上传音频文件；**ASR 历史** 检索过往记录。服务商与凭证在「设置」中配置（火山引擎 / 通义千问）。

### P2P 多设备

两台设备安装 EffiSuite 后，在 **P2P 设备** 面板完成配对（IP 直连或广播请求），即可建立端到端加密通道：镜像同步会话/插件，或直接向另一台设备派发任务。

## 📁 数据存储

应用数据统一存放在系统数据目录下的 `effisuite/` 子目录：

```
<app_data_dir>/effisuite/
├── config.json          # Agent / 模型 / 主题 / ASR 配置
├── conversations/       # 会话历史
├── memory/              # RAG 记忆索引与向量
├── asr/                 # 语音转写记录与音频
├── skills/              # 已安装技能
├── schedules/           # 定时任务
└── plugins/             # 插件
```

## 🛠 常见问题

- **DeepSeek 计费不准确？** — 本项目对 rig-core 做了 vendored 补丁（`vendor/rig-core/PATCH.md`），以正确解析 DeepSeek 的 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` 缓存计费字段。升级 rig-core 时需重新应用补丁。
- **模型配置改了没生效？** — 工具/配置变更后 agent 会在下一次消息时自动重建，无需重启；若遇到异常可重启应用。
- **WebView 相关内容无法显示？** — 前端使用 `markstream-vue` 渲染 Markdown，内置 Mermaid 流程图与 KaTeX 数学公式支持。

## ☕ 支持

如果你觉得 EffiSuite 对你有帮助，欢迎扫描下方收款码请我喝杯咖啡 ☕ 你的支持是我持续维护下去的动力！

![微信收款码](docs/mm_facetoface_collect_qrcode.png)

## 📄 License

[MIT](https://github.com/EffiSuite/EffiSuite)（见 `Cargo.toml` 的 `license` 字段）

---

*EffiSuite 仍处于早期开发阶段（v0.1.0），功能与接口可能随迭代调整。*
