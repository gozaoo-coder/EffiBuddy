` 传参约定
   - 返回 op_id、支持范围读取等行为承诺
5. 错误信息、调试信息不进 description。

## 四、parameters 精简规范

1. 每个参数 `description` 压到**短语（≤15 字）**，不重复 `default` 值已表达的内容。
2. **保持 JSON schema 结构**：`type` / `items` / `properties` / `required` / `enum` / `default` 不变，
   否则影响工具调用参数反序列化。
3. 顶层冗余 `description`（如 `"description": "无参数"`）可删。
4. **不改 `Args` 结构体**（字段名 / 类型 / serde 属性），不改 `call()` 逻辑，不改常量。

## 五、改动范围与验证

- 只改 `fn description()` / `fn parameters()` 的返回文本。
- 每个文件改后 `cargo build -p effisuite-agent` 需通过；涉及工具的单测需保持绿色。
- 复查工具总数不变（55+），name 不变，功能不变。

## 六、清单（按文件，desc+params 排序）

| 优先级 | 文件 | 现状 | 精简后预期 |
|---|---|---|---|
| P0 | todo_write.rs | 4912 | ≤1800 |
| P0 | edit_file/tool.rs | 3090 | ≤1100 |
| P0 | edit_file/regex_tool.rs | 2686 | ≤1000 |
| P0 | ask_user.rs | 2572 | ≤900 |
| P0 | search_file.rs | 2068 | ≤900 |
| P0 | generate_video.rs | 2003 | ≤800 |
| P1 | shell.rs | 1916 | ≤750 |
| P1 | schedule_tool.rs | 1585 | ≤650 |
| P1 | sub_agent.rs | 1562 | ≤700 |
| P1 | read_file.rs | 1459 | ≤700 |
| P1 | edit_file/revise_tool.rs | 1446 | ≤600 |
| P1 | glob_tool.rs | 1400 | ≤600 |
| P1 | write_file.rs | 1344 | ≤700 |
| P1 | image_gen.rs | 1332 | ≤550 |
| P2 | model_manager.rs | 1246 | ≤500 |
| P2 | 其余（call_model/agent_pool/dispatch/search_memory/list_files/delete_file/undo/web_search/display_image/open_preview/pin_memory/asr/plugin/skill_tools/notify/search_history/set_title/compact/web_fetch/get_time） | 各 ≤1150 | 各 ≤450 |
| — | **合计** | **47,945** | **≤21,000** |
