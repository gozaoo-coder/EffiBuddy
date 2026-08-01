# 子 Agent 调用全流程设计

> 主 agent 通过 `sub_agent` 工具召唤子 agent 执行任务，全过程实时推送到前端展示。

## 一、调用链总览

```
用户提问
  └─ 主 agent（RigAgent，含全部工具）
       ├─ [工具] manage_model    → 增删改查模型列表 / 激活模型（配置持久化 + 版本号 bump）
       ├─ [工具] call_model      → 一次性调用任意已保存模型（无工具单轮，返回文本）
       ├─ [工具] image_gen       → 图像生成（支持 model_id 指定图像模型）
       └─ [工具] sub_agent       → 召唤子 agent（本次设计核心）
            ├─ SubAgentManager 创建/复用会话（独立消息历史，不落盘）
            ├─ 子 agent = 独立 RigAgent（可指定模型/指令/工具白名单）
            ├─ 流式执行：token / 工具调用 / 图片 全量事件推送
            └─ 返回最终回复文本给主 agent
```

## 二、sub_agent 工具参数

| 参数 | 说明 |
|------|------|
| `prompt` | 交给子 agent 的任务（必填） |
| `session_id` | 会话 id；留空自动生成；**复用同一 id 可多轮继续**；`close=true` 时执行后关闭 |
| `name` | 显示名（前端卡片标题） |
| `model_id` | 子 agent 使用的模型（manage_model list 可查）；缺省与主 agent 相同 |
| `instructions` | 任务指令（追加到子 agent 系统提示词） |
| `tools` | 工具白名单；缺省=默认工具集（排除 set_title/display_image/image_gen）；空数组=无工具 |
| `close` | 执行后关闭会话 |

## 三、SubAgentManager 运行时约束

- **嵌套深度**：`AtomicUsize` 计数（进入 +1 / 退出 -1），上限 2 层（主 agent→子→孙），防无限递归
- **会话数**：上限 16，超出淘汰最久未用（LRU by last_active）
- **消息数**：单会话上限 40 条，超出丢弃最早消息（内存态，不落盘）
- **锁安全**：会话 agent 用 `Arc<RigAgent>`，执行时**锁外运行**——避免嵌套 sub_agent 时写锁死锁
- **子 agent 能力**：拥有与主 agent 相同的句柄集（memory/pinned/skills/working_dir/模型管理…），
  因此子 agent 也可以 manage_model / call_model / 再召唤子 agent（受深度限制）

## 四、事件流（后端 → 前端）

`SubAgentEvent`（`sub-agent-event` 事件，带 conversation_id 过滤）：

```
started      content=任务原文          ← 卡片创建 + 显示任务
token        content=文本增量          ← 回复流式累积
tool_call    tool_name+arguments      ← 卡片内工具行（spinner）
tool_result  content=输出, is_error   ← 工具行结果
attachment   content=ImageGenOutput   ← 子 agent 生成的图片（卡片内渲染）
done         content=最终回复全文      ← 卡片完成态 + 结果高亮条
error        content=错误信息          ← 卡片错误态
```

发射链路：`SubAgentManager.emit` → 事件回调（Tauri 层构造，闭包捕获 AppHandle 槽位）
→ `app_handle.emit("sub-agent-event", ev)` → 前端 `ChatWindow` 监听。

## 五、前端布局（SubAgentCard.vue）

```
┌──────────────────────────────────────────────┐
│ 🤖 代码审查员 · gpt-4o  [深度2]  ● 运行中    ▾ │ ← 头部（点击折叠）
├──────────────────────────────────────────────┤
│ 任务 审查 src/main.rs 的安全性                │ ← 任务原文
│ 🔧 read_file …                    ●          │ ← 工具行（名/参/态）
│ 🔧 shell …                         ✓          │
│ 🖼 [图片缩略图 120px]                          │ ← 生成图片
│ 回复 流式文本…                                │ ← 回复（max-height 滚动）
│ ✅ 结果：…… [复制]                            │ ← 完成态结果条
└──────────────────────────────────────────────┘
折叠态：单行摘要「🤖 代码审查员 · gpt-4o · 完成 · 结果前 48 字…」
运行中强制展开；完成/出错可折叠。深色/浅色自适应（CSS 变量）。
```

## 六、模型管理（manage_model）

- `list`：全部模型 + 当前激活标记（对话/图像）
- `save`：新增/更新（id 空自动生成），持久化
- `delete`：删除 + 清空激活标记
- `activate`：chat → 激活对话模型；image_gen → 激活图像模型

**懒重建机制**：工具修改配置 → `config_rev` bump → 下次 `send_message` 时
`ensure_agent_synced` 比对 `agent_rev`，不一致则重建 agent + 同步 image_gen_config 句柄
+ emit `agent-backend-changed`（前端模型面板自动刷新）。当前轮对话不受影响。

## 七、与 call_model 的分工

| | call_model | sub_agent |
|---|---|---|
| 轮数 | 单轮 | 多轮（session_id 续聊） |
| 工具 | 无 | 有（白名单可控） |
| 历史 | 无 | 独立会话历史 |
| 事件 | 无 | 全流程实时推送 |
| 适用 | 交叉验证/子任务 | 复杂独立任务/委派 |
