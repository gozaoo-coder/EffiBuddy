# EffiSuite 插件架构（Plugin Architecture）

> 版本：API 1.0 ｜ 状态：设计基线，随实现演进

EffiSuite 的插件体系以「**声明式 manifest**」为核心：插件包不携带可执行代码，
只声明「我能贡献什么」（左栏按钮 / 页面 / 命令）。安全性由**后端只读解析 +
白名单校验 + 路径防护**保证，前端只消费解析后的声明式数据。

---

## 1. 插件包结构

一个插件 = 一个 ZIP 包，解压到 `<appdata>/plugins/<safe_id>/`：

```
my-plugin/
├── plugin.json        # manifest（必需；也接受 manifest.json / effisuite.json）
└── ...                # 附带资源（当前版本不执行、不加载插件内脚本）
```

`safe_id` = `safe_plugin_path_segment(plugin_id)`：`<owner>/<name>` 中的 `/` 替换为 `__`，
防止目录穿越。

## 2. Manifest 格式

`plugin.json` 字段：

```jsonc
{
  "api_version": "1.0",                 // 主版本必须与 MANIFEST_API_VERSION 兼容
  "id": "owner/my-plugin",              // 必须与安装记录 id 一致（校验）
  "name": "my-plugin",
  "display_name": "我的插件",
  "version": "1.0.0",
  "description": "……",
  "author": "owner",
  "permissions": ["config.read", "ui.page"],   // 白名单内才能通过校验
  "contributions": {
    "rail": [                            // 左栏一按钮
      {
        "id": "open-todo",
        "label": "我的待办",
        "icon": "book",                  // 可选
        "section": "main",               // main | bottom，默认 main
        "action": { "type": "open-page", "pageId": "my-plugin/todo" }
        // 或 { "type": "command", "command": "my-plugin/someCmd" }
      }
    ],
    "pages": [                           // 页面 / 页签 / 路由 / 组件
      {
        "id": "todo",
        "title": "我的待办",
        "icon": "book",
        "route": "/todo",                // 可选
        "entry": "builtin"               // builtin（前端注册表）| file（预留）
      }
    ],
    "commands": [                        // 注册给 agent 的命令 / skill
      { "id": "list", "name": "todo_list", "description": "列出待办事项" }
    ]
  }
}
```

### 权限白名单（KNOWN_PERMISSIONS）

| 权限 | 含义 |
|---|---|
| `config.read` / `config.write` / `config.delete` | 读写/删除插件自己的配置命名空间 |
| `agent.command` / `agent.skill` | 注册命令 / 技能给 agent |
| `ui.rail` / `ui.page` | 贡献左栏按钮 / 页面 |
| `files.read` | 预留：读取插件包内文件 |
| `shell` | 预留：执行命令（当前不授予） |

> 声明了白名单之外的能力 → manifest 校验失败，插件贡献整体跳过。

## 3. 生命周期（Lifecycle）

```
安装 ──► 激活（运行期持续） ──► 卸载
```

| 阶段 | 触发 | 动作 |
|---|---|---|
| **安装** | `clawhub_install_plugin` | 拉详情 → 下载 ZIP → `spawn_blocking` 解压（zip-slip 防护）到 `<plugins_dir>/<safe_id>/` → 写 `plugin_store` 元数据 → `sync_plugin_skills` 注册其命令为 agent 技能 → emit `plugins-changed` |
| **激活** | 启动 / 任意时刻 | `list_plugin_contributions` 遍历已安装插件：路径防护 → `load_manifest` → `validate`（id 匹配 / 权限白名单 / 贡献 id 唯一）→ `build_contribution_set` 组装带前缀的贡献。前端据此渲染左栏按钮与页面 |
| **启动同步** | 应用启动 | 技能索引重建后调用 `sync_plugin_skills`（幂等 upsert + 清除已失效插件技能） |
| **更新** | 预留 | 同安装路径（覆盖解压目录 + 更新元数据），贡献自动刷新 |
| **卸载** | `clawhub_uninstall_plugin` | 删 `plugin_store` 记录 → `cleanup_plugin_config` 清配置 → `sync_plugin_skills` 清除该插件技能 → emit `plugins-changed` |

### 无活动插件的运行时生命周期

插件**没有自己的运行进程 / 脚本**。所谓「激活」= 贡献集合被后端解析、被前端消费：
- 左栏按钮 → 点击触发 `open-page`（跳转插件页签）或 `command`（预留）
- 页面 → 前端 `usePluginPages` 注册表按页面 id 解析组件（`builtin` 内置；`file` 预留动态加载）
- 命令 → `sync_plugin_skills` 注册为 SkillStore 中 `source="plugin"` 的技能，
  进入 `list_installed_skills` 与 RAG 技能注入，agent 可理解并调用

## 4. 配置存储（appdata 命名空间隔离）

- 位置：`<appdata>/plugin_configs/<safe_id>.json`
- 命令：`get_plugin_config` / `set_plugin_config` / `delete_plugin_config` / `get_plugin_config_all`
- 隔离：写前先 `ensure_plugin_installed` 校验插件已安装；键值 JSON 任意序列化
- 卸载时 `cleanup_plugin_config` 自动清理

## 5. 安全声明（Safety Model）

1. **不执行插件代码**：插件贡献全部为声明式数据（JSON manifest），
   ClawHub 安装只做「下载 + 解压 + 元数据落盘」，不加载/运行任何脚本。
2. **只读解析 + 严格校验**：`load_manifest` 解析、`PluginManifest::validate` 校验
   （API 版本兼容 / id 匹配 / 贡献 id 非空唯一 / 权限白名单）。
3. **路径防护**：解压目录必须在 `<plugins_dir>` 根下（`starts_with` 检查）；
   `safe_plugin_path_segment` 防目录穿越。
4. **权限最小化**：manifest 声明的能力必须落在 `KNOWN_PERMISSIONS` 白名单内，
   超出即拒绝（当前默认不授予 `shell`）。
5. **配置命名空间隔离**：未安装插件无法读写任何配置；配置按插件隔离落盘。
6. **技能同步可回滚**：插件卸载 → 其注册的技能被 `sync_plugin_skills` 清除，
   索引自动重建，不残留。

## 6. 内置示例

`builtin_contributions()` 提供 EffiSuite 内置「我的待办」示例：
- 左栏按钮 `effisuite/user-todo`（open-page）
- 页面 `effisuite/user-todo` → 前端注册表解析到 `UserTodoPage.vue`
- 命令 `effisuite/user-todo/list` → 注册为 agent 技能 `plugin:effisuite/user-todo/list`

## 7. 模块地图

| 位置 | 职责 |
|---|---|
| `core/src/plugin_manifest.rs` | manifest 模型 / 校验 / 加载 / 贡献组装 |
| `core/src/plugin_config.rs` | 插件配置命名空间存储（appdata） |
| `core/src/plugin_store.rs` | 已安装插件元数据存储 |
| `app/src/commands/plugins.rs` | 贡献汇总命令 / 配置命令 / `sync_plugin_skills` |
| `app/src/commands/clawhub.rs` | 安装 / 卸载（解压、清理、触发技能同步） |
| `app/src/composables/usePluginContributions.ts` | 前端贡献拉取 + `plugins-changed` 订阅 |
| `app/src/composables/usePluginPages.ts` | 页面 id → Vue 组件注册表 |
| `app/src/components/plugin-pages/UserTodoPage.vue` | 内置示例页面 |
