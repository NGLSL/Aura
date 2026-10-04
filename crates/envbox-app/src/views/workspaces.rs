use crate::app::EnvBoxApp;
use crate::message::Message;
use iced::widget::{button, column, pick_list, row, scrollable, text, text_input};
use iced::{Element, Fill};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChoice {
    pub id: Uuid,
    pub name: String,
}
impl std::fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({})",
            self.name,
            self.id.to_string().split('-').next().unwrap_or("")
        )
    }
}

pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let state = &app.workspaces;
    let mut items = column![
        text("持久工作区").size(22),
        text("Compatibility 工作区通过 Supervisor 启动并管理。文件与 Registry 规则仅预览，当前仍使用宿主存储；支持严格 UDP、TCP 和 DoT DNS，DoH 尚未启用。"),
        row![
            button("新建").on_press(Message::WorkspaceNew),
            button("刷新").on_press(Message::WorkspaceRefresh)
        ]
        .spacing(8),
    ]
    .spacing(14);
    if let Some(error) = &state.error {
        items = items.push(text(format!("读取或保存失败：{error}")));
    }
    for workspace in &state.document.containers {
        items = items.push(
            button(text(format!("{} · {}", workspace.name, workspace.id)))
                .on_press(Message::WorkspaceSelect(workspace.id)),
        );
    }
    let options: Vec<ProfileChoice> = app
        .profiles
        .iter()
        .map(|profile| ProfileChoice {
            id: profile.id,
            name: profile.name.clone(),
        })
        .collect();
    let selected = options
        .iter()
        .find(|profile| profile.id == state.draft.profile_id)
        .cloned();
    if state.selected.is_some() && selected.is_none() {
        items = items.push(text(format!(
            "环境配置 {} 已不存在或无法读取；工作区及已有快照保留，新准备会失败。",
            state.draft.profile_id
        )));
    }
    items = items.push(
        column![
            text(if state.selected.is_some() {
                "编辑工作区"
            } else {
                "新建工作区"
            })
            .size(18),
            text_input("名称", &state.draft.name).on_input(Message::WorkspaceName),
            pick_list(options, selected, |profile| Message::WorkspaceProfile(
                profile.id
            ))
            .placeholder("选择环境配置"),
            text("模式：Compatibility · 尚未启用文件或 Registry Overlay"),
            row![
                button("保存").on_press(Message::WorkspaceSave),
                button("取消修改").on_press(Message::WorkspaceCancel)
            ]
            .spacing(8),
        ]
        .spacing(10),
    );
    let management = &app.workspace_management;
    let applications: Vec<ProfileChoice> = app
        .applications
        .iter()
        .map(|application| ProfileChoice {
            id: application.id,
            name: application.name.clone(),
        })
        .collect();
    let application = applications
        .iter()
        .find(|value| Some(value.id) == management.application_id)
        .cloned();
    let mut run_button = button("创建快照并运行");
    if !management.busy
        && !state.dirty()
        && state.selected.is_some()
        && application.is_some()
        && management.pending_run.is_none()
        && management.generation.is_some()
    {
        run_button = run_button.on_press(Message::WorkspaceRun);
    }
    let mut refresh_button = button("刷新 Supervisor 实例");
    let mut stop_all = button("停止此工作区全部实例");
    if !management.busy {
        refresh_button = refresh_button.on_press(Message::WorkspaceList);
        if state.selected.is_some() && management.generation.is_some() {
            stop_all = stop_all.on_press(Message::WorkspaceStopAll);
        }
    }
    items = items.push(text("工作区运行 · Supervisor").size(18));
    items = items.push(
        pick_list(applications, application, |value| {
            Message::WorkspaceApplication(value.id)
        })
        .placeholder("选择已保存应用"),
    );
    items = items.push(row![run_button, refresh_button, stop_all].spacing(8));
    items = items.push(text("不支持 Packaged、控制台 broker 或关闭子进程继承的应用；由服务器验证入口资格。停止范围由服务器的工作区实例集合决定。"));
    if management.busy {
        items = items.push(text("正在处理 Supervisor 请求…"));
    }
    if let Some(error) = &management.error {
        items = items.push(text(format!("Supervisor 请求失败：{error}")));
    }
    if let Some(notice) = &management.notice {
        items = items.push(text(notice));
    }
    if let Some(generation) = &management.generation {
        items = items.push(text(format!("当前 Supervisor generation：{generation}")));
    }
    if let Some(pending) = &management.pending_run {
        items = items.push(text(format!(
            "待确认实例：{} · 工作区 {}",
            pending.instance_id, pending.container_id
        )));
        let mut query = button("查询原实例运行结果");
        if !management.busy && Some(pending.container_id) == state.selected {
            query = query.on_press(Message::WorkspaceRunStatus);
        }
        items = items.push(query);
    }
    if let Some(workspace_id) = state.selected {
        for view in management.workspace_instances(workspace_id) {
            let result = &view.result;
            let mut stop = button("停止实例");
            if !management.busy
                && Some(result.supervisor_generation.as_str()) == management.generation.as_deref()
                && result.state != "TrackingLost"
            {
                stop = stop.on_press(Message::WorkspaceStop(result.instance_id));
            }
            items = items.push(
                column![
                    text(format!(
                        "{} · {} · PID {}",
                        result.instance_id, result.state, result.root_pid
                    )),
                    text(format!(
                        "Application {} · generation {}",
                        result.application_id, result.supervisor_generation
                    )),
                    text(format!(
                        "{} · 入口保证 {} · 存储规则执行 {}",
                        result.mode, result.entry_guarantee, result.storage_policy_enforced
                    )),
                    text(format!(
                        "配置身份 {} · 快照 {}",
                        result.configuration_id, result.snapshot_digest
                    )),
                    text(format!("当前 Job 进程：{:?}", view.process_ids)),
                    text(result.error.as_deref().unwrap_or("")),
                    stop,
                ]
                .spacing(6),
            );
        }
    }
    use envbox_core::storage_policy::{StorageAction, StorageTarget};
    items = items.push(text("存储规则 · 仅配置预览，尚未生效").size(18));
    items = items.push(text("未匹配写入策略为拒绝；实际对象、重解析点和别名仍需后续 backend 验证。Compatibility 目前仍使用宿主存储。"));
    for (index, rule) in state.draft.storage_policy.rules.iter().enumerate() {
        items = items.push(
            row![
                text(format!("{} · {} · {}", rule.target, rule.action, rule.path)).width(Fill),
                button("移除规则").on_press(Message::WorkspaceRuleRemove(index)),
            ]
            .spacing(8),
        );
    }
    items = items.push(
        column![
            pick_list(
                vec![StorageTarget::FileDirectory, StorageTarget::RegistrySubtree],
                Some(state.rule_draft.target),
                Message::WorkspaceRuleTarget
            ),
            text_input(
                "绝对应用目录或 HKCU\\Software\\厂商\\应用",
                &state.rule_draft.path
            )
            .on_input(Message::WorkspaceRulePath),
            pick_list(
                vec![
                    StorageAction::IsolatedWrite,
                    StorageAction::SharedReadOnly,
                    StorageAction::SharedReadWrite,
                    StorageAction::Deny
                ],
                Some(state.rule_draft.action),
                Message::WorkspaceRuleAction
            ),
            button("添加规则").on_press(Message::WorkspaceRuleAdd),
        ]
        .spacing(10),
    );
    scrollable(items.width(Fill)).into()
}
