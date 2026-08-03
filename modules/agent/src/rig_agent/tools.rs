//! `RigAgent::build_agent` 内部的工具注册逻辑
//!
//! 每次调用 `chat` / `chat_stream` 时重建一个带工具的 `rig_core::agent::Agent`。
//! 装配工具时按以下分组顺序注册：
//! 1. 会话内检索 / 本地能力 / 图像 / 标题 / 文件名匹配 / 内容搜索 / 语义搜索 / 网络搜索
//! 1.5. 正则编辑 / 历史查看修订 / 撤回（edit_file_regex / edit_revise / edit_undo）
//!      —— 仅在 `edit_history` 注入时
//! 2. 模型管理（manage_model / call_model）—— 仅在 `model_manager` 注入时
//! 3. 子 agent（sub_agent）—— 仅在 `sub_agents` 注入时
//! 4. 跨会话记忆检索（search_memory）—— 仅在 `memory` 注入时
//! 5. 永久记忆（pin_memory / list_pinned_memories / delete_pinned_memory）
//! 6. 技能管理（list_installed_skills / get_skill_detail / enable_skill / uninstall_skill）
//! 7. ClawHub（search_clawhub_skills / install_clawhub_skill）
//! 8. 插件卸载（uninstall_plugin）
//! 9. 用户交互（ask_user / notify_user / open_preview）—— 仅在 `event_bus` 注入时
//! 10. 定时任务（schedule）—— 仅在 `scheduled_task_store` 注入时
//! 11. 待办列表（todo_write）
//! 12. 视频生成（generate_video）

use std::path::PathBuf;
use std::sync::Arc;

use rig_core::client::CompletionClient;
use rig_core::providers::openai;

use crate::tools::{
    AsrTool, AskUserTool, CallModelTool, DeleteFileTool, DeletePinnedMemoryTool, DisplayImageTool,
    DispatchRemoteTaskTool, EditFileRegexTool, EditFileTool, EditReviseTool, EditUndoTool,
    GenerateVideoTool, GetAsrRecordTool, GetSkillDetailTool, GetTimeTool, GlobTool, GrepTool,
    ImageGenTool, InstallClawHubSkillTool, ListAsrTool, ListFilesTool, ListInstalledSkillsTool,
    ListPinnedMemoriesTool, ManageModelTool, NotifyUserTool, OpenPreviewTool, PinMemoryTool,
    ReadFileTool, ScheduleTool, SearchAsrTool, SearchClawHubSkillsTool, SearchCodebaseTool,
    SearchFileTool, SearchHistoryTool, SearchMemoryTool, EnableSkillTool, SetTitleTool,
    ShellSessionKillTool, ShellSessionListTool, ShellSessionReadTool, ShellSessionSendTool,
    ShellSessionStartTool, ShellSessionWaitTool, ShellTool, SubAgentTool, TodoWriteTool,
    UninstallPluginTool,
    UninstallSkillTool, WebFetchTool, WebSearchTool, WriteFileTool,
};

use super::RigAgent;

impl RigAgent {
    /// 构建一个带工具的 agent（每次调用重新构建，零成本）
    ///
    /// 装配工具时，所有工具共享同一份 history Arc 与 current_conversation_id Arc，
    /// 确保 LLM 调用 search_history / search_memory 时看到的是最新上下文。
    /// `cwd` 快照在调用前从 `working_dir` RwLock 读取，注入到文件/shell 工具。
    ///
    /// 返回类型用关联类型 `<openai::CompletionsClient as CompletionClient>::CompletionModel`，
    /// 即 `GenericCompletionModel<OpenAICompletionsExt>`，统一所有 OpenAI 兼容 provider。
    pub(super) fn build_agent(
        &self,
        cwd: Option<PathBuf>,
    ) -> rig_core::agent::Agent<<openai::CompletionsClient as CompletionClient>::CompletionModel>
    {
        let builder = self
            .client
            .agent(&self.model_name)
            .preamble(&self.preamble);

        if self.enable_tools {
            // 工具过滤：白名单（None=全部）+ 排除列表（子 agent 默认排除 set_title 等）
            let want = |name: &str| {
                self.tool_allowlist
                    .as_ref()
                    .is_none_or(|a| a.iter().any(|t| t == name))
                    && !self.exclude_tools.iter().any(|t| t == name)
            };
            // 注册会话内检索工具：每次 build 都重新创建工具实例，但它们共享 history
            let search = SearchHistoryTool::new(Arc::clone(&self.history));
            let time = GetTimeTool::new(Arc::clone(&self.history));
            // 本地能力工具：读文件、列目录、执行 shell（agent-reach/browser-act）、抓网页
            // 注入工作区 cwd（若有），相对路径以此为准
            let read_file = match &cwd {
                Some(p) => ReadFileTool::with_cwd(p.clone()),
                None => ReadFileTool::new(),
            };
            let write_file = match &cwd {
                Some(p) => WriteFileTool::with_cwd(p.clone()),
                None => WriteFileTool::new(),
            };
            let edit_file = match &cwd {
                Some(p) => EditFileTool::with_cwd(p.clone()),
                None => EditFileTool::new(),
            };
            // 注入编辑历史句柄：启用 op_id 编号与撤回/修订能力
            let edit_file = match &self.edit_history {
                Some(h) => edit_file.with_history(h.clone()),
                None => edit_file,
            };
            // 正则编辑工具：与 edit_file 共享同一份 history（仅在 history 注入时注册）
            let edit_file_regex = self.edit_history.as_ref().map(|h| {
                let t = match &cwd {
                    Some(p) => EditFileRegexTool::with_cwd(p.clone()),
                    None => EditFileRegexTool::new(),
                };
                t.with_history(h.clone())
            });
            // 历史查看/修订工具：history 必填，仅在注入时注册
            let edit_revise = self.edit_history.as_ref().map(|h| match &cwd {
                Some(p) => EditReviseTool::with_cwd(p.clone(), h.clone()),
                None => EditReviseTool::new(h.clone()),
            });
            // 撤回工具：history 必填，仅在注入时注册
            let edit_undo = self
                .edit_history
                .as_ref()
                .map(|h| EditUndoTool::new(h.clone()));
            let search_file = match &cwd {
                Some(p) => SearchFileTool::with_cwd(p.clone()),
                None => SearchFileTool::new(),
            };
            let delete_file = match &cwd {
                Some(p) => DeleteFileTool::with_cwd(p.clone()),
                None => DeleteFileTool::new(),
            };
            let list_files = match &cwd {
                Some(p) => ListFilesTool::with_cwd(p.clone()),
                None => ListFilesTool::new(),
            };
            let shell = match &cwd {
                Some(p) => ShellTool::with_cwd(p.clone()),
                None => ShellTool::new(),
            };
            let web_fetch = WebFetchTool::new();
            // 图像生成工具：共享 image_gen_config 句柄，调用时读取最新配置。
            // 用户切换到 kind=ImageGen 的模型时由 Tauri 命令层更新 config，
            // LLM 可主动调用此工具为用户生成图片；支持 model_id 指定图像模型。
            let image_gen = ImageGenTool::new(
                Arc::clone(&self.image_gen_config),
                self.attachments_dir.clone(),
            );
            let image_gen = match &self.model_manager {
                Some(mm) => image_gen.with_models(Arc::clone(&mm.config)),
                None => image_gen,
            };
            // 图片展示工具：让 LLM 把已有图片（本地路径或 URL）推送到聊天框。
            // 与 image_gen（生成新图）互补，复用 attachments_dir 落盘。
            let display_image = match &cwd {
                Some(p) => DisplayImageTool::with_cwd(p.clone(), self.attachments_dir.clone()),
                None => DisplayImageTool::new(self.attachments_dir.clone()),
            };
            // 会话标题设置工具：LLM 据此为会话生成/更新标题（≤25 字）
            // 共享 store 与 current_conversation_id 句柄，调用时直接落盘
            let set_title = SetTitleTool::new(
                Arc::clone(&self.store),
                Arc::clone(&self.current_conversation_id),
            );

            // 文件名模式匹配工具：glob 语法（**/*.rs 等），与 read_file/edit_file 配合
            // LLM 拿到文件列表后再读取/编辑，避免盲目猜测路径
            let glob = match &cwd {
                Some(p) => GlobTool::with_cwd(p.clone()),
                None => GlobTool::new(),
            };
            // 正则内容搜索工具：跨文件按正则匹配，返回命中行（带行号、上下文）
            // 与 search_file（关键词精确匹配）互补，与 grep CLI 对齐
            let grep = match &cwd {
                Some(p) => GrepTool::with_cwd(p.clone()),
                None => GrepTool::new(),
            };
            // 语义代码搜索工具：自然语言查询 → Top-N 代码块
            // 与 search_file（精确匹配）互补，适合"我想找做 X 的代码但不知道函数名"
            let search_codebase = match &cwd {
                Some(p) => SearchCodebaseTool::with_cwd(p.clone()),
                None => SearchCodebaseTool::new(),
            };
            // 网络搜索工具：共享 web_search_config 句柄，用户切换引擎后下次调用即生效
            let web_search = WebSearchTool::new(Arc::clone(&self.web_search_config));

            let mut b = builder
                .tool(search)
                .tool(time)
                .tool(read_file)
                .tool(write_file)
                .tool(edit_file)
                .tool(search_file)
                .tool(delete_file)
                .tool(list_files)
                .tool(shell)
                .tool(web_fetch)
                .tool(image_gen)
                .tool(display_image)
                .tool(set_title)
                .tool(glob)
                .tool(grep)
                .tool(search_codebase)
                .tool(web_search);

            // 正则编辑 / 历史查看修订 / 撤回工具：仅在 edit_history 注入时注册
            // 与 edit_file 共享同一份 history，使 op_id 在 4 个工具间互通
            if let Some(t) = edit_file_regex {
                b = b.tool(t);
            }
            if let Some(t) = edit_revise {
                b = b.tool(t);
            }
            if let Some(t) = edit_undo {
                b = b.tool(t);
            }

            // 模型管理与调用工具：仅在 ModelManagerHandle 可用时注册
            // manage_model：agent 自主增删改查模型列表 / 激活模型
            // call_model：一次性调用任意已保存模型（无工具单轮）
            if let Some(mm) = &self.model_manager {
                if want("manage_model") {
                    let manage = ManageModelTool::new(Arc::clone(mm));
                    b = b.tool(manage);
                }
                if want("call_model") {
                    let call = CallModelTool::new(Arc::clone(&mm.config));
                    b = b.tool(call);
                }
            }

            // 子 agent 工具：仅在 SubAgentManager 可用时注册
            if let Some(sa) = &self.sub_agents {
                if want("sub_agent") {
                    let sub = SubAgentTool::new(Arc::clone(sa));
                    b = b.tool(sub);
                }
            }

            // 跨会话记忆检索工具：仅在 MemoryIndex 可用时注册
            if let Some(memory) = &self.memory {
                let search_memory = SearchMemoryTool::new(
                    Arc::clone(memory),
                    Arc::clone(&self.current_conversation_id),
                );
                b = b.tool(search_memory);
            }

            // 永久记忆工具：仅在 PinnedMemoryStore 可用时注册
            // 让 LLM 能在用户说"请记住..."时主动调用 pin_memory 落盘
            if let Some(pinned) = &self.pinned_memory {
                let pin = PinMemoryTool::new(
                    Arc::clone(pinned),
                    Arc::clone(&self.current_conversation_id),
                );
                let list_pinned = ListPinnedMemoriesTool::new(Arc::clone(pinned));
                let delete_pinned = DeletePinnedMemoryTool::new(Arc::clone(pinned));
                b = b.tool(pin).tool(list_pinned).tool(delete_pinned);
            }

            // 技能管理工具：仅在 skill_index + skill_store 同时可用时注册
            // 让 LLM 自主列出 / 查询 / 启用 / 卸载本地已安装技能（替代旧 apply_skill 命令）
            if let (Some(idx), Some(store)) = (&self.skill_index, &self.skill_store) {
                let list_skills = ListInstalledSkillsTool::new(Arc::clone(idx));
                let get_skill = GetSkillDetailTool::new(store.clone());
                let enable_skill = EnableSkillTool::new(
                    store.clone(),
                    Arc::clone(&self.store),
                    Arc::clone(&self.current_conversation_id),
                );
                let uninstall_skill = UninstallSkillTool::new(store.clone(), Arc::clone(idx));
                b = b
                    .tool(list_skills)
                    .tool(get_skill)
                    .tool(enable_skill)
                    .tool(uninstall_skill);
            }

            // ClawHub 工具：仅在 clawhub_client 可用时注册
            // 让 LLM 在本地无匹配技能时主动从 ClawHub 搜索 + 安装
            // install_clawhub_skill 额外依赖 skill_store / skill_index / skills_dir，
            // 任一缺失则只暴露 search_clawhub_skills（agent 可推荐 slug 但不能直接安装）
            if let Some(client) = &self.clawhub_client {
                let search_clawhub = SearchClawHubSkillsTool::new(client.clone());
                b = b.tool(search_clawhub);
                if let (Some(store), Some(idx), Some(dir)) =
                    (&self.skill_store, &self.skill_index, &self.skills_dir)
                {
                    let install_clawhub = InstallClawHubSkillTool::new(
                        client.clone(),
                        store.clone(),
                        Arc::clone(idx),
                        dir.clone(),
                    );
                    b = b.tool(install_clawhub);
                }
            }

            // 插件管理工具：仅在 plugin_store 可用时注册
            if let Some(plugin_store) = &self.plugin_store {
                let uninstall_plugin = UninstallPluginTool::new(plugin_store.clone());
                b = b.tool(uninstall_plugin);
            }

            // 用户交互工具：依赖 event_bus（前端通信通道）。
            // ask_user：向用户提出选择题，等待回答（用于方案确认/偏好选择）
            // notify_user：通知用户审核文件或查看重要信息
            // open_preview：请求前端打开预览 URL（如本地 dev server）
            // 三者均通过 BusEvent 与前端通信，event_bus 为 None 时不注册
            // （工具调用会返回友好错误，但更优做法是直接不注册避免 LLM 误调用）
            if let Some(bus) = &self.event_bus {
                if want("ask_user") {
                    let ask = AskUserTool::new(
                        Some(Arc::clone(bus)),
                        Arc::clone(&self.current_conversation_id),
                    );
                    b = b.tool(ask);
                }
                if want("notify_user") {
                    let notify = NotifyUserTool::new(
                        Some(Arc::clone(bus)),
                        Arc::clone(&self.current_conversation_id),
                    );
                    b = b.tool(notify);
                }
                if want("open_preview") {
                    let open = OpenPreviewTool::new(
                        Some(Arc::clone(bus)),
                        Arc::clone(&self.current_conversation_id),
                    );
                    b = b.tool(open);
                }
            }

            // 定时任务管理工具：依赖 scheduled_task_store（与 Tauri 调度器共享同一份）
            // 让 LLM 通过 cron 表达式创建/更新/暂停/删除定时任务
            if let Some(store) = &self.scheduled_task_store {
                if want("schedule") {
                    let schedule = ScheduleTool::new(Arc::clone(store));
                    b = b.tool(schedule);
                }
            }

            // 待办列表工具：优先使用每会话 TodoStore（写入后持久化到当前会话 + 通知前端）；
            // 无 store 时回退到共享内存状态（主 agent 与子 agent 可视化任务进度）
            if want("todo_write") {
                let mut todo = match &self.todo_state {
                    Some(state) => TodoWriteTool::with_state(Arc::clone(state)),
                    None => TodoWriteTool::new(),
                };
                if let Some(store) = &self.todo_store {
                    todo = todo.with_persistence(
                        store.clone(),
                        Arc::clone(&self.current_conversation_id),
                        self.event_bus.clone(),
                    );
                }
                b = b.tool(todo);
            }

            // 视频生成工具：共享 video_gen_config 句柄（与 image_gen 同模式），
            // 用户切换到 kind=video_gen 的模型时由 Tauri 命令层更新 config，
            // LLM 可主动调用此工具为用户生成视频；支持 model_id 指定视频模型
            if want("generate_video") {
                let video_gen = GenerateVideoTool::new(
                    Arc::clone(&self.video_gen_config),
                    self.attachments_dir.clone(),
                );
                let video_gen = match &self.model_manager {
                    Some(mm) => video_gen.with_models(Arc::clone(&mm.config)),
                    None => video_gen,
                };
                b = b.tool(video_gen);
            }

            // ASR 语音转写工具集：仅在 asr_service 注入时注册。
            // transcribe_audio：转写本地音频文件，自动生成摘要
            // search_asr_records：按关键词搜索已转写的 ASR 记录
            // list_asr_records：列出最近的 ASR 转写记录
            // get_asr_record：获取指定 ASR 记录的完整转写文本
            // 流式录音 API 不作为 LLM 工具暴露，由 Tauri 命令层直接暴露给前端
            if let Some(asr) = &self.asr_service {
                let transcribe = AsrTool::new(Arc::clone(asr));
                let search_asr = SearchAsrTool::new(Arc::clone(asr));
                let list_asr = ListAsrTool::new(Arc::clone(asr));
                let get_asr = GetAsrRecordTool::new(Arc::clone(asr));
                b = b.tool(transcribe).tool(search_asr).tool(list_asr).tool(get_asr);
            }

            // 远端任务派发工具：仅在 remote_task_dispatcher 注入时注册（P2P 镜像模式）。
            // dispatch_remote_task：list 列出在线已配对设备，dispatch 向指定设备派发自然语言任务。
            // 用 trait object 避免 agent crate 依赖 effisuite-p2p（依赖倒置）。
            // P2pManager 实现了 RemoteTaskDispatcher trait，由 Tauri 命令层在 build_agent 时注入。
            if let Some(dispatcher) = &self.remote_task_dispatcher {
                if want("dispatch_remote_task") {
                    let dispatch = DispatchRemoteTaskTool::new(Arc::clone(dispatcher));
                    b = b.tool(dispatch);
                }
            }
            // 后台命令会话工具集：仅在 shell_sessions 管理器注入时注册。
            // 让 LLM 启用常驻 cmd/sh 会话（后台静默运行）并持续交互，
            // 会话输出实时推送前端底栏便签展示 AI 工作状态。
            if let Some(mgr) = &self.shell_sessions {
                if want("shell_session_start") {
                    b = b.tool(ShellSessionStartTool::new(Arc::clone(mgr)));
                }
                if want("shell_session_send") {
                    b = b.tool(ShellSessionSendTool::new(Arc::clone(mgr)));
                }
                if want("shell_session_read") {
                    b = b.tool(ShellSessionReadTool::new(Arc::clone(mgr)));
                }
                if want("shell_session_wait") {
                    b = b.tool(ShellSessionWaitTool::new(Arc::clone(mgr)));
                }
                if want("shell_session_list") {
                    b = b.tool(ShellSessionListTool::new(Arc::clone(mgr)));
                }
                if want("shell_session_kill") {
                    b = b.tool(ShellSessionKillTool::new(Arc::clone(mgr)));
                }
            }

            // 运行时 agent 公共会话交流池工具集：仅在 AgentPoolStore 注入时注册。
            // 多会话并行时用于跨会话协作：pool_report 登记长任务、pool_lookup 查询
            // 活跃长任务、pool_at @ 目标 agent（async/await）、pool_reply 回复收件箱。
            // 子 agent（pool_sub_agent_id 非空）同样注册，身份按 sa:<session_id> 推导。
            if let Some(pool) = &self.agent_pool {
                let ctx = crate::tools::PoolCtx {
                    pool: pool.clone(),
                    conv_id: Arc::clone(&self.current_conversation_id),
                    sub_agent_id: self.pool_sub_agent_id.clone(),
                    sub_agent_name: self.pool_sub_agent_name.clone(),
                    store: Some(Arc::clone(&self.store)),
                    event_bus: self.event_bus.clone(),
                };
                if want("pool_report") {
                    b = b.tool(crate::tools::PoolReportTool::new(ctx.clone()));
                }
                if want("pool_lookup") {
                    b = b.tool(crate::tools::PoolLookupTool::new(ctx.clone()));
                }
                if want("pool_at") {
                    b = b.tool(crate::tools::PoolAtTool::new(ctx.clone()));
                }
                if want("pool_reply") {
                    b = b.tool(crate::tools::PoolReplyTool::new(ctx));
                }
            }

            let b = b.default_max_turns(usize::MAX);
            super::user_interrupt::attach_user_inject_hook(
                b,
                self.pending_user_messages.clone(),
                Arc::clone(&self.current_conversation_id),
            )
            .build()
        } else {
            super::user_interrupt::attach_user_inject_hook(
                builder,
                self.pending_user_messages.clone(),
                Arc::clone(&self.current_conversation_id),
            )
            .build()
        }
}
}
