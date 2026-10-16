use crate::app::EnvBoxApp;
use crate::font;
use crate::message::Message;
use crate::theme::{self, DANGER_TEXT, INK, INK_2, MUTED, SUCCESS_TEXT};
use crate::widgets::{field_label, short_id};
use iced::widget::{button, column, container, pick_list, row, text, tooltip};
use iced::{Alignment, Element, Fill, Padding};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChoice {
    pub id: Uuid,
    pub name: String,
}
impl std::fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
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
            name: if app.applications.iter().filter(|other| other.name == application.name).count() > 1 {
                format!("{} · {}", application.name, short_id(application.id))
            } else {
                application.name.clone()
            },
        })
        .collect();
    let application = applications
        .iter()
        .find(|value| Some(value.id) == management.application_id)
        .cloned();
    let mut run_button = action("启动应用").style(theme::primary_btn);
    if !management.busy
        && !state.dirty()
        && state.selected.is_some()
        && application.is_some()
        && management.pending_run.is_none()
        && management.generation.is_some()
    {
        run_button = run_button.on_press(Message::WorkspaceRun);
    }
    let mut refresh_button = action("刷新状态");
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
    if management
        .application_id
        .and_then(|id| {
            app.applications
                .iter()
                .find(|application| application.id == id)
        })
        .is_some_and(|application| application.supports_chrome_data())
    {
        let detection = pick_list(
            envbox_core::DetectionSite::ALL,
            None::<envbox_core::DetectionSite>,
            Message::WorkspaceDetect,
        )
        .placeholder("打开检测页")
        .text_size(12)
        .font(font::ui_font())
        .padding(Padding::from([7, 8]))
        .width(Fill)
        .style(theme::pick_style)
        .menu_style(theme::pick_menu);
        // The action handler applies the same saved/idle checks as snapshot startup.
        if !management.busy {
            runs = runs.push(detection);
        }
    }
    let active_count = app.profile_draft.id.map(|id| {
        management.profile_instances(id)
            .filter(|view| !matches!(view.result.state.as_str(), "Exited" | "Stopped" | "Failed"))
            .count()
    }).unwrap_or(0);
    let mut toolbar = row![refresh_button].spacing(8);
    if active_count > 1 {
        toolbar = toolbar.push(stop_all);
    }
    runs = runs.push(toolbar);
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
    if let Some(pending) = &management.pending_run {
        if app
            .profile_draft
            .id
            .is_some_and(|profile_id| management.pending_belongs_to_profile(profile_id))
        {
            runs = runs.push(technical_info(
                "启动结果待确认".into(),
                format!(
                    "实例 ID：{}\n运行范围 ID：{}",
                    pending.instance_id, pending.container_id
                ),
                INK_2,
            ));
            let mut query = action("查询启动结果");
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
            runs = runs.push(instance_card(app, view));
        }
    }
    runs = runs.push(technical_info(
        "运行说明".into(),
        "应用继续使用宿主文件、GPU 和用户权限，不提供安全隔离。Profile 代理仅用于 Chrome 与出口检测。\nDNS 仅覆盖支持的 Windows API；应用自带 DNS 和未注入的浏览器进程可能读取宿主信息。\n工作区不支持 Packaged、控制台 broker 或关闭子进程继承的应用；停止操作仅作用于此配置关联的实例。".into(),
        MUTED,
    ));
    container(runs)
        .padding(12)
        .width(Fill)
        .style(theme::panel_card_style)
        .into()
}

fn instance_card<'a>(app: &'a EnvBoxApp, view: &'a envbox_supervisor::RunView) -> Element<'a, Message> {
    let result = &view.result;
    let management = &app.workspace_management;
    let expanded = management.diagnostics_instance == Some(result.instance_id);
    let running = result.state == "Running";
    let lost = result.state == "TrackingLost";
    let terminal = matches!(result.state.as_str(), "Exited" | "Stopped" | "Failed");
    let (status, color) = match result.state.as_str() {
        "Running" => ("运行中", SUCCESS_TEXT),
        "Starting" => ("启动中", INK_2),
        "Stopping" => ("正在停止", MUTED),
        "Exited" => ("已退出", MUTED),
        "Stopped" => ("已停止", MUTED),
        "Failed" => ("启动失败", DANGER_TEXT),
        "TrackingLost" => ("跟踪已中断", DANGER_TEXT),
        other => (other, MUTED),
    };
    let application_name = app.applications.iter()
        .find(|application| application.id == result.application_id)
        .map(|application| application.name.clone())
        .unwrap_or_else(|| "应用已删除".into());
    let mut card = column![
        row![
            text(application_name).size(14).color(INK).font(font::name_font()).width(Fill),
            text(status).size(13).color(color).font(font::ui_font()),
        ].spacing(8).align_y(Alignment::Center),
    ].spacing(10);
    let observed = result.member_runtimes.iter()
        .filter(|member| member.environment_facts.is_some()).count();
    if running {
        let summary = match &result.environment_facts {
            Some(facts) if facts.config_complete && facts.profile_matches_snapshot => "主进程已载入匹配配置",
            Some(_) => "主进程配置未完整匹配",
            None => "暂未取得主进程环境信息",
        };
        let summary = if result.entry_guarantee == envbox_supervisor::ENTRY_CHROME_COMPATIBILITY {
            format!("{summary} · 兼容启动")
        } else { summary.into() };
        card = card.push(text(summary).size(12).color(INK_2).font(font::ui_font()));
        if observed < result.known_members.len() {
            card = card.push(text("部分子进程尚无环境观测").size(12).color(MUTED).font(font::ui_font()));
        }
    } else if lost {
        let recovery = if result.entry_guarantee == envbox_supervisor::ENTRY_CHROME_COMPATIBILITY {
            "浏览器可能仍在运行。请先关闭对应浏览器，再刷新状态；恢复条件见诊断详情。"
        } else {
            "应用可能仍在运行，暂时无法控制。请刷新状态，或查看诊断详情了解原因。"
        };
        card = card.push(text(recovery)
            .size(12).color(DANGER_TEXT).font(font::ui_font()));
    }
    let details_label = if expanded { "收起详情" }
        else if result.error.is_some() && !lost { "查看失败原因" }
        else { "诊断详情" };
    let mut controls = row![action(details_label)
        .on_press(Message::WorkspaceDiagnostics(result.instance_id))].spacing(8);
    if !terminal {
        let mut stop = action("停止").style(theme::danger_btn);
        if !management.busy
            && Some(result.supervisor_generation.as_str()) == management.generation.as_deref()
            && !lost
        {
            stop = stop.on_press(Message::WorkspaceStop(result.instance_id));
        }
        controls = controls.push(stop);
    }
    card = card.push(controls);
    if expanded {
        card = card.push(instance_diagnostics(view));
    }
    container(card).padding(14).width(Fill).style(theme::inner_card_style).into()
}

fn instance_diagnostics(view: &envbox_supervisor::RunView) -> Element<'_, Message> {
    let result = &view.result;
    let observed = result.member_runtimes.iter()
        .filter(|member| member.environment_facts.is_some()).count();
    let mut details = column![
        text(format!("运行方式：{}\n入口保证：{}", result.mode,
            envbox_supervisor::entry_guarantee_label(&result.entry_guarantee)))
            .size(12).color(INK_2).font(font::ui_font()),
        text(format!("已记录环境观测：{} / {} 个进程\n当前 Job 进程：{:?}",
            observed, result.known_members.len(), view.process_ids))
            .size(12).color(MUTED).font(font::ui_font()),
    ].spacing(10);
    if let Some(facts) = &result.environment_facts {
        let hooks = facts.hooks.iter()
            .map(|hook| format!("{}：{} 个入口", hook.group, hook.attached_api_count))
            .collect::<Vec<_>>().join("\n");
        details = details.push(text(format!("最近根进程观测\n配置完整：{}\nProfile 与快照匹配：{}\n已安装 Hook（不代表所有读取入口均已验证）\n{}",
            if facts.config_complete { "是" } else { "否" },
            if facts.profile_matches_snapshot { "是" } else { "否" },
            if hooks.is_empty() { "未报告" } else { &hooks }))
            .size(12).color(MUTED).font(font::ui_font()));
    }
    if result.entry_guarantee == envbox_supervisor::ENTRY_CHROME_COMPATIBILITY {
        details = details.push(text("兼容启动核验根进程 Runtime，不使用严格入口门禁。沙箱子进程可能读取本机信息。退出 Aura 界面不会停止浏览器；Supervisor 重启后只观测原 Job，不恢复控制。关闭浏览器并刷新，确认 Job 归零后才解除跟踪中断；服务离线期间 Job 已消失时会保留中断状态。")
            .size(12).color(MUTED).font(font::ui_font()));
    }
    if let Some(error) = &result.error {
        details = details.push(text(error).size(12).color(DANGER_TEXT).font(font::ui_font())
            .wrapping(iced::widget::text::Wrapping::Glyph));
    }
    details = details.push(text(format!("根进程 PID：{}\n实例：{}\nApplication：{}\nSupervisor generation：{}\n配置身份：{}\n快照：{}",
        result.root_pid, result.instance_id, result.application_id, result.supervisor_generation,
        result.configuration_id, result.snapshot_digest))
        .size(12).color(MUTED).font(font::ui_font())
        .wrapping(iced::widget::text::Wrapping::Glyph));
    container(details).padding(Padding::from([10, 0])).width(Fill).into()
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
