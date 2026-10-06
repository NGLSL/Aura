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
        text("环境容器").size(22),
        text("容器保存 Profile 信息视图与运行记录，每次运行使用不可变快照。目标程序继续使用宿主文件、网络、GPU 和用户权限。"),
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
            "环境配置 {} 已不存在或无法读取；环境容器及已有快照保留，新准备会失败。",
            state.draft.profile_id
        )));
    }
    items = items.push(
        column![
            text(if state.selected.is_some() {
                "编辑环境容器"
            } else {
                "新建环境容器"
            })
            .size(18),
            text_input("名称", &state.draft.name).on_input(Message::WorkspaceName),
            pick_list(options, selected, |profile| Message::WorkspaceProfile(
                profile.id
            ))
            .placeholder("选择环境配置"),
            text("DNS Host 模式使用 Windows 宿主 DNS；VirtualView 模式下，受支持的 DNS API 按 Profile 配置使用 UDP、TCP、DoT、DoH，Strict 模式禁止失败后回退宿主 DNS。"),
            text("应用自带 DNS 和不能注入的浏览器 renderer 可能读取宿主信息；Profile 不改变公网出口 IP。"),
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
    let mut stop_all = button("停止此环境容器全部实例");
    if !management.busy {
        refresh_button = refresh_button.on_press(Message::WorkspaceList);
        if state.selected.is_some() && management.generation.is_some() {
            stop_all = stop_all.on_press(Message::WorkspaceStopAll);
        }
    }
    items = items.push(text("环境容器运行 · Supervisor").size(18));
    items = items.push(
        pick_list(applications, application, |value| {
            Message::WorkspaceApplication(value.id)
        })
        .placeholder("选择已保存应用"),
    );
    items = items.push(row![run_button, refresh_button, stop_all].spacing(8));
    items = items.push(text("不支持 Packaged、控制台 broker 或关闭子进程继承的应用；由服务器验证入口资格。停止范围由服务器的环境容器实例集合决定。"));
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
            "待确认实例：{} · 环境容器 {}",
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
            let environment_observation = match &result.environment_facts {
                Some(facts) => {
                    let hooks = facts
                        .hooks
                        .iter()
                        .map(|hook| format!("{} {} 个入口", hook.group, hook.attached_api_count))
                        .collect::<Vec<_>>()
                        .join("；");
                    format!(
                        "最近根进程观测：配置完整 {} · Profile 与快照匹配 {} · 已安装 Hook：{}",
                        if facts.config_complete { "是" } else { "否" },
                        if facts.profile_matches_snapshot {
                            "是"
                        } else {
                            "否"
                        },
                        if hooks.is_empty() {
                            "未报告"
                        } else {
                            &hooks
                        }
                    )
                }
                None => "根进程环境信息：未观测（包括旧运行记录）".into(),
            };
            let observed_members = result
                .member_runtimes
                .iter()
                .filter(|member| member.environment_facts.is_some())
                .count();
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
                        "{} · 入口保证 {}",
                        result.mode, result.entry_guarantee
                    )),
                    text(format!(
                        "配置身份 {} · 快照 {}",
                        result.configuration_id, result.snapshot_digest
                    )),
                    text(environment_observation),
                    text(format!(
                        "已记录成员环境观测：{} / {} · 未观测 {} · Hook 安装信息不代表所有读取入口已验证",
                        observed_members,
                        result.known_members.len(),
                        result.known_members.len().saturating_sub(observed_members)
                    )),
                    text(format!("当前 Job 进程：{:?}", view.process_ids)),
                    text(result.error.as_deref().unwrap_or("")),
                    stop,
                ]
                .spacing(6),
            );
        }
    }
    scrollable(items.width(Fill)).into()
}
