# Tools 实现三方对比报告 & effibuddy 优化计划

> 分析对象：
> - **claude-code-cli**：`tools/`（每工具一目录）+ `Tool.ts` + `assembleToolPool`
> - **opencode**：`packages/core/src/tool/`（`tool.ts`/`registry.ts`/`tools.ts`）
> - **effibuddy (EffiSuite)**：`modules/agent/src/tools/` + `rig_agent/tools.rs`

---

## 一、架构范式

| | claude-code-cli | opencode | effibuddy (EffiSuite) |
|---|---|---|---|
| 语言/运行时 | TypeScript + Zod | TypeScript + Effect + Schema | Rust + rig |
| 工具定义形式 | 富对象（object literal），20+ 元数据字段 | `Tool.make()` 声明式工厂 | `impl Tool` trait（NAME/Error/Args/Output/description/parameters/call） |
| 注册机制 | 集中表驱动 `getTools/getAllBaseTools` + `assembleToolPool`(合并 MCP) | `ToolRegistry` Service：scope 生命周期 + materialize + settle | `build_agent` 手写 660+ 行 `if let + .tool()` 链式装配，11 个条件分组 |
| 输入校验 | Zod schema | Schema 解码 | serde `Deserialize`（仅入参） |
| 输出校验 | 无 | Schema 编码验证 → `ToolFailure` | 无 |
| 权限层 | `canUseTool` + deny rules + permission context | `PermissionV2` ruleset + wildcard + `assert()` | **无**（仅 `resolve_path` cwd 软锚定） |
| 错误处理 | 分级消息渲染 | Effect 错误 → 统一 `ToolFailure` | `thiserror`，字符串化返回 |
| 并发/只读/危险元数据 | `isConcurrencySafe`/`isReadOnly`/`isDestructive`/`interruptBehavior` | Effect 并发调度 + `withPermission` | **无** |
| 结果超限 | `maxResultSizeChars` 截断落盘 | `outputStore` 绑定 | 手动 max_bytes 截断 |
| 工具延迟加载 | `ToolSearch` + `searchHint` | — | **无** |
| 测试 | 有 | 有 | 每工具 `#[tokio::test]`（质量最好 ✅） |

---

## 二、典型工具实现剖析（read 类）

### opencode `read.ts`
- 输入/输出都用 **Effect Schema** 强类型约束；
- `toModelOutput` 把图片 base64 结果转成模型可见的多模态消息；
- 执行全程 `Effect.gen`，依赖（FileSystem/Image/Permission）通过 **Layer 依赖注入**；
- 双重 `permission.assert`（外部目录 + 读动作）；错误 `mapError` 收敛成一条 `ToolFailure` 消息。

### claude-code-cli `FileReadTool.ts`
- Lambda 度极高：PDF/图片/notebook/设备文件防护、token 估算、memory 加持、权限 wildcard 匹配，全部由复用 util 组合；
- 元数据丰富但 **单个文件 1183 行**，偏重。

### effibuddy `read_file.rs`（315 行，含 4 个单测）
- `impl Tool` trait：`name/description/parameters(call)/call`；参数用 serde；`with_cwd` 注入工作区；
- 自带**行号渲染 + 行范围精读 + 256KB 截断 + 尾部标注**，效果对齐 search_file/edit_file；
- **缺失**：输出 schema/长度校验、权限、结果落盘。

---

## 三、effibuddy 现状瓶颈

1. **装配硬编码**：`build_agent` 660+ 行手写 `if let/.tool()`，新增工具要改 imports + 分组注册，易漏、不利于测试与权限控制。
2. **无统一注册表/元数据**：工具没有"只读/危险/并发"等标记，无法做权限拦截、批量过滤。
3. **无权限层**：仅路径软锚定，无 deny rules / 外部目录审批（claude 与 opencode 都有）。
4. **无输出校验/超限落盘**：长结果只能内存截断，大文件读入费 token。
5. **无延迟加载/工具搜索**：工具集 60+ 全量注入，prompt 重；claude 有 ToolSearch 兜底名。

---

## 四、effibuddy 优化计划（按 ROI 排序）

### P0 — 高价值低风险
1. **引入统一 ToolRegistry**（Rust 侧）
   - 建 `ToolKind`（Read/Write/Destructive/Shell/Model/User）元数据 trait，`ANY` 默认；
   - 用 `vec![Box<dyn ToolDef>] + builder` 取代手写链条，`build_agent` 收敛为声明式列表 + 按 name 过滤。
   - **收益**：新增工具一处注册、代码量减半、可测。

2. **描述/参数聚合 + prompt 降负**
   - 仿 claude `assembleToolPool` 的稳定排序：按分组常量排序，避免每次 build prompt 序抖动；
   - 为超大工具（sub_agent 33KB 描述）提供精简描述模式（对外只给契约）。

### P1 — 中等价值
3. **只读/危险元数据 + 简单权限层**
   - 标记 destructive / shell / 写文件工具，支持 `exclude_tools` 之外的 deny-list；
   - 对 shell / delete / edit 类可提"路径不得命中目标"的轻量审批钩子（本地优先，无前端则拒绝）。

4. **长结果落盘模式**
   - read_file / search 超限时，仿 `maxResultSizeChars`：把内容写临时文件，返回 `[已写入 /tmp/xxx，可用 read_file 分段读]`，省 token。

### P2 — 规划项
5. **输出 Schema 提示词降噪 + 结果校验**（可后置）
6. **工具注解宏**：为 `impl Tool` 加 derive 宏自动生成 JSON Schema（复用 `parameters()`），减少手写参数描述漂移。

---

## 五、结论
effibuddy 工具**功能完备度最高、测试最扎实**，最大短板是**装配与元数据基础设施**（无注册表/权限/元数据），其次是**结果处理**（无落盘）。按 P0→P2 推进可在不破坏现有 11 组依赖注入的前提下，先做声明式注册表 + 元数据，再补权限与落盘，整体向 opencode 的 registry + claude 的元数据范式靠拢，同时保留 rig trait 的 Rust 强类型优势。