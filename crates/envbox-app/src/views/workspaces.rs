use crate::app::EnvBoxApp;
use crate::font;
use crate::message::Message;
use crate::theme::{self, DANGER_TEXT, INK, INK_2, MUTED};
use crate::widgets::{field_label, short_id};
use iced::widget::{button, column, container, pick_list, row, text, tooltip};
use iced::{Element, Fill, Padding};
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

// Kept for internal routing compatibility; the public entry is Profile detail.
pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    run_panel(app)
}

pub fn run_panel(app: &EnvBoxApp) -> Element<'_, Message> {
    let state = &app.workspaces;
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
    let mut run_button = action("创建快照并运行").style(theme::primary_btn);
    if !management.busy
        && !state.dirty()
        && state.selected.is_some()
        && application.is_some()
        && management.pending_run.is_none()
        && management.generation.is_some()
    {
        run_button = run_button.on_press(Message::WorkspaceRun);
    }
    let mut refresh_button = action("刷新运行状态");
    let mut stop_all = action("停止全部实例").style(theme::danger_btn);
    if !management.busy {
        refresh_button = refresh_button.on_press(Message::WorkspaceList);
        if state.selected.is_some() && management.generation.is_some() {
            stop_all = stop_all.on_press(Message::WorkspaceStopAll);
        }
    }
    let mut runs = column![
        text("使用此配置运行")
            .size(17)
            .color(INK)
            .font(font::name_font()),
        field_label("已保存应用")
    ]
    .spacing(10);
    runs = runs.push(
        pick_list(applications, application, |value| {
            Message::WorkspaceApplication(value.id)
        })
        .placeholder("选择要运行的应用")
        .text_size(13)
        .font(font::ui_font())
        .padding(Padding::from([10, 12]))
        .width(Fill)
        .style(theme::pick_style)
        .menu_style(theme::pick_menu),
    );
    runs = runs.push(run_button.width(Fill));
    runs = runs.push(row![refresh_button, stop_all].spacing(8));
    if let Some(error) = &state.error {
        runs = runs.push(
            text(format!("准备运行配置失败：{error}"))
                .size(13)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        );
    }
    if app.applications.is_empty() {
        runs = runs.push(
            text("尚无已保存应用，请先在“应用”页面添加。")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }
    if state.selected.is_none() {
        runs = runs.push(
            text("当前配置的运行范围尚未准备好，请重新选择配置或刷新。")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }
    if management.busy {
        runs = runs.push(
            text("正在处理运行管理请求…")
                .size(13)
                .color(MUTED)
                .font(font::ui_font()),
        );
    } else if management.generation.is_none() {
        runs = runs.push(
            text("尚未取得运行管理状态，请刷新。")
                .size(12)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }
    if let Some(error) = &management.error {
        runs = runs.push(
            text(format!("运行管理请求失败：{error}"))
                .size(13)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        );
    }
    if let Some(notice) = &management.notice {
        runs = runs.push(text(notice).size(13).color(INK_2).font(font::ui_font()));
    }
    if let Some(generation) = &management.generation {
        runs = runs.push(technical_info(
            "运行管理会话标识".into(),
            format!("Supervisor generation：{generation}"),
            MUTED,
        ));
    }
    if let Some(pending) = &management.pending_run {
        if app
            .profile_draft
            .id
            .is_some_and(|profile_id| management.pending_belongs_to_profile(profile_id))
        {
            runs = runs.push(technical_info(
                format!("实例 {} 的运行结果待确认", short_id(pending.instance_id)),
                format!(
                    "实例 ID：{}\n运行范围 ID：{}",
                    pending.instance_id, pending.container_id
                ),
                INK_2,
            ));
            let mut query = action("查询原实例运行结果");
            if !management.busy {
                query = query.on_press(Message::WorkspaceRunStatus);
            }
            runs = runs.push(query);
        } else {
            runs = runs.push(
                text("另一环境配置的启动结果待确认，请切回原配置查询。")
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
            );
        }
    }
    if let Some(profile_id) = app.profile_draft.id {
        for view in management.profile_instances(profile_id) {
            let result = &view.result;
            let mut stop = action("停止实例").style(theme::danger_btn);
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
            let application_name = app
                .applications
                .iter()
                .find(|application| application.id == result.application_id)
                .map(|application| application.name.clone())
                .unwrap_or_else(|| format!("应用 {}", short_id(result.application_id)));
            let details = format!(
                "实例 ID：{}\nApplication：{}\nSupervisor generation：{}\n配置身份：{}\n快照：{}",
                result.instance_id,
                result.application_id,
                result.supervisor_generation,
                result.configuration_id,
                result.snapshot_digest
            );
            let mut instance = column![
                    text(format!(
                        "{} · {} · PID {}",
                        application_name, result.state, result.root_pid
                    )).size(14).color(INK).font(font::name_font()),
                    technical_info(format!("实例 {} · 标识详情", short_id(result.instance_id)), details, MUTED),
                    text(format!(
                        "{} · 入口保证 {}",
                        result.mode, result.entry_guarantee
                    )).size(12).color(INK_2).font(font::ui_font()),
                    text(environment_observation).size(12).color(INK_2).font(font::ui_font()),
                    text(format!(
                        "已记录成员环境观测：{} / {} · 未观测 {} · Hook 安装信息不代表所有读取入口已验证",
                        observed_members,
                        result.known_members.len(),
                        result.known_members.len().saturating_sub(observed_members)
                    )).size(12).color(MUTED).font(font::ui_font()),
                    text(format!("当前 Job 进程：{:?}", view.process_ids)).size(12).color(MUTED).font(font::ui_font()),
                ]
                .spacing(8);
            if let Some(error) = &result.error {
                instance = instance.push(
                    text(error)
                        .size(13)
                        .color(DANGER_TEXT)
                        .font(font::ui_font()),
                );
            }
            instance = instance.push(stop);
            runs = runs.push(
                container(instance)
                    .padding(14)
                    .width(Fill)
                    .style(theme::inner_card_style),
            );
        }
    }
    runs = runs.push(technical_info(
        "支持范围（悬停查看）".into(),
        "每次启动保存独立快照；应用继续使用宿主文件、网络、GPU 和用户权限，公网出口 IP 保持不变。\n配置 DNS 仅覆盖支持的 Windows DNS API；应用自带 DNS 和无法注入的浏览器 renderer 仍可能读取宿主信息。\n不支持 Packaged、控制台 broker 或关闭子进程继承的应用。停止操作仅针对此配置实际关联的运行实例。".into(),
        MUTED,
    ));
    container(runs)
        .padding(12)
        .width(Fill)
        .style(theme::panel_card_style)
        .into()
}

fn action<'a>(label: &'static str) -> iced::widget::Button<'a, Message> {
    button(text(label).size(13).font(font::ui_font()))
        .padding(Padding::from([9, 14]))
        .style(theme::secondary_btn)
}

fn technical_info(label: String, detail: String, color: iced::Color) -> Element<'static, Message> {
    tooltip(
        text(label).size(12).color(color).font(font::ui_font()),
        text(detail).size(12).color(INK_2).font(font::ui_font()),
        tooltip::Position::Top,
    )
    .style(theme::inner_card_style)
    .into()
}
