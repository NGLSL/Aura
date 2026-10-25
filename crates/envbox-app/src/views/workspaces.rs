use crate::app::EnvBoxApp;
use crate::font;
use crate::message::Message;
use crate::theme::{self, DANGER_TEXT, INK, INK_2, MUTED, SUCCESS_TEXT};
use crate::theme::{TEXT_BODY, TEXT_CAPTION, TEXT_SECTION};
use iced::widget::{button, column, container, row, text, tooltip};
use iced::{Alignment, Element, Fill, Padding};

// Legacy route renders management only; startup belongs to the Applications page.
pub fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let management = &app.workspace_management;
    let mut runs = column![text("后台管理实例")
        .size(TEXT_SECTION)
        .color(INK)
        .font(font::name_font())]
    .spacing(10);
    if management.busy {
        runs = runs.push(
            text("正在更新运行管理状态…")
                .size(TEXT_BODY)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }
    if let Some(error) = &management.error {
        runs = runs.push(
            text(format!("运行管理请求失败：{error}"))
                .size(TEXT_BODY)
                .color(DANGER_TEXT)
                .font(font::ui_font())
                .wrapping(iced::widget::text::Wrapping::Glyph),
        );
    }
    if let Some(notice) = &management.notice {
        runs = runs.push(
            text(notice)
                .size(TEXT_BODY)
                .color(INK_2)
                .font(font::ui_font()),
        );
    }
    if let Some(pending) = &management.pending_run {
        // Recovery is global: an application filter or deleted Profile must not hide it.
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
    }
    let mut groups = Vec::new();
    for view in &management.instances {
        if app
            .instance_filter
            .is_none_or(|id| view.result.application_id == id)
            && !groups.contains(&view.result.profile_id)
        {
            groups.push(view.result.profile_id);
        }
    }
    for profile_id in groups {
        let instances: Vec<_> = management
            .profile_instances(profile_id)
            .filter(|view| {
                app.instance_filter
                    .is_none_or(|id| view.result.application_id == id)
            })
            .collect();
        let controllable = instances
            .iter()
            .filter(|view| {
                Some(view.result.supervisor_generation.as_str()) == management.generation.as_deref()
                    && !matches!(
                        view.result.state.as_str(),
                        "TrackingLost" | "Exited" | "Stopped" | "Failed"
                    )
            })
            .count();
        let mut heading = row![text(format!("环境：{}", profile_label(app, profile_id)))
            .size(TEXT_BODY)
            .color(INK_2)
            .font(font::ui_font())
            .width(Fill)]
        .spacing(8)
        .align_y(Alignment::Center);
        if app.instance_filter.is_none() && controllable > 1 {
            let mut stop = action("停止此环境实例").style(theme::danger_btn);
            if !management.busy {
                stop = stop.on_press(Message::ManagedProfileStop(profile_id));
            }
            heading = heading.push(stop);
        }
        runs = runs.push(heading);
        for view in instances {
            runs = runs.push(instance_card(app, view));
        }
    }
    container(runs)
        .padding(12)
        .width(Fill)
        .style(theme::panel_card_style)
        .into()
}

fn instance_card<'a>(
    app: &'a EnvBoxApp,
    view: &'a envbox_supervisor::RunView,
) -> Element<'a, Message> {
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
    let application_name = app
        .applications
        .iter()
        .find(|application| application.id == result.application_id)
        .map(|application| application.name.clone())
        .unwrap_or_else(|| "应用已删除".into());
    let profile_name = profile_label(app, result.profile_id);
    let mut card = column![row![
        text(application_name)
            .size(TEXT_SECTION)
            .color(INK)
            .font(font::name_font())
            .width(Fill),
        text(status)
            .size(TEXT_BODY)
            .color(color)
            .font(font::ui_font()),
    ]
    .spacing(8)
    .align_y(Alignment::Center),]
    .spacing(10);
    card = card.push(
        text(format!("环境：{profile_name}"))
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
    );
    let observed = result
        .member_runtimes
        .iter()
        .filter(|member| member.environment_facts.is_some())
        .count();
    if running {
        let summary = match &result.environment_facts {
            Some(facts) if facts.config_complete && facts.profile_matches_snapshot => {
                "主进程已载入匹配配置"
            }
            Some(_) => "主进程配置未完整匹配",
            None => "暂未取得主进程环境信息",
        };
        let summary = if result.entry_guarantee == envbox_supervisor::ENTRY_CHROME_COMPATIBILITY {
            format!("{summary} · 兼容启动")
        } else {
            summary.into()
        };
        card = card.push(
            text(summary)
                .size(TEXT_BODY)
                .color(INK_2)
                .font(font::ui_font()),
        );
        if observed < result.known_members.len() {
            card = card.push(
                text("部分子进程尚无环境观测")
                    .size(TEXT_CAPTION)
                    .color(MUTED)
                    .font(font::ui_font()),
            );
        }
    } else if lost {
        let recovery = if result.entry_guarantee == envbox_supervisor::ENTRY_CHROME_COMPATIBILITY {
            "浏览器可能仍在运行。请先关闭对应浏览器，再刷新状态；恢复条件见诊断详情。"
        } else {
            "应用可能仍在运行，暂时无法控制。请刷新状态，或查看诊断详情了解原因。"
        };
        card = card.push(
            text(recovery)
                .size(TEXT_BODY)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        );
    }
    let details_label = if expanded {
        "收起详情"
    } else if result.error.is_some() && !lost {
        "查看失败原因"
    } else {
        "诊断详情"
    };
    let mut controls =
        row![action(details_label).on_press(Message::WorkspaceDiagnostics(result.instance_id))]
            .spacing(8);
    if !terminal {
        let mut stop = action("停止").style(theme::danger_btn);
        if !management.busy
            && Some(result.supervisor_generation.as_str()) == management.generation.as_deref()
            && !lost
        {
            stop = stop.on_press(Message::ManagedInstanceStop(result.instance_id));
        }
        controls = controls.push(stop);
    }
    card = card.push(controls);
    if expanded {
        card = card.push(instance_diagnostics(view));
    }
    container(card)
        .padding(14)
        .width(Fill)
        .style(theme::inner_card_style)
        .into()
}

fn instance_diagnostics(view: &envbox_supervisor::RunView) -> Element<'_, Message> {
    let result = &view.result;
    let observed = result
        .member_runtimes
        .iter()
        .filter(|member| member.environment_facts.is_some())
        .count();
    let mut details = column![
        text(format!(
            "运行方式：{}\n入口保证：{}",
            result.mode,
            envbox_supervisor::entry_guarantee_label(&result.entry_guarantee)
        ))
        .size(TEXT_CAPTION)
        .color(INK_2)
        .font(font::ui_font()),
        text(format!(
            "已记录环境观测：{} / {} 个进程\n当前 Job 进程：{:?}",
            observed,
            result.known_members.len(),
            view.process_ids
        ))
        .size(TEXT_CAPTION)
        .color(MUTED)
        .font(font::ui_font()),
    ]
    .spacing(10);
    if let Some(facts) = &result.environment_facts {
        let hooks = facts
            .hooks
            .iter()
            .map(|hook| format!("{}：{} 个入口", hook.group, hook.attached_api_count))
            .collect::<Vec<_>>()
            .join("\n");
        details = details.push(text(format!("最近根进程观测\n配置完整：{}\nProfile 与快照匹配：{}\n已安装 Hook（不代表所有读取入口均已验证）\n{}",
            if facts.config_complete { "是" } else { "否" },
            if facts.profile_matches_snapshot { "是" } else { "否" },
            if hooks.is_empty() { "未报告" } else { &hooks }))
            .size(TEXT_CAPTION).color(MUTED).font(font::ui_font()));
    }
    if result.entry_guarantee == envbox_supervisor::ENTRY_CHROME_COMPATIBILITY {
        details = details.push(text("兼容启动核验根进程 Runtime，不使用严格入口门禁。沙箱子进程可能读取本机信息。退出 Aura 界面不会停止浏览器；Supervisor 重启后只观测原 Job，不恢复控制。关闭浏览器并刷新，确认 Job 归零后才解除跟踪中断；服务离线期间 Job 已消失时会保留中断状态。")
            .size(TEXT_CAPTION).color(MUTED).font(font::ui_font()));
    }
    if let Some(error) = &result.error {
        details = details.push(
            text(error)
                .size(TEXT_CAPTION)
                .color(DANGER_TEXT)
                .font(font::ui_font())
                .wrapping(iced::widget::text::Wrapping::Glyph),
        );
    }
    details = details.push(text(format!("根进程 PID：{}\n实例：{}\nApplication：{}\nSupervisor generation：{}\n配置身份：{}\n快照：{}",
        result.root_pid, result.instance_id, result.application_id, result.supervisor_generation,
        result.configuration_id, result.snapshot_digest))
        .size(TEXT_CAPTION).color(MUTED).font(font::ui_font())
        .wrapping(iced::widget::text::Wrapping::Glyph));
    container(details)
        .padding(Padding::from([10, 0]))
        .width(Fill)
        .into()
}

fn action<'a>(label: &'static str) -> iced::widget::Button<'a, Message> {
    button(text(label).size(TEXT_BODY).font(font::ui_font()))
        .padding(Padding::from([9, 14]))
        .style(theme::secondary_btn)
}

fn technical_info(label: String, detail: String, color: iced::Color) -> Element<'static, Message> {
    tooltip(
        text(label)
            .size(TEXT_CAPTION)
            .color(color)
            .font(font::ui_font()),
        text(detail)
            .size(TEXT_CAPTION)
            .color(INK_2)
            .font(font::ui_font()),
        tooltip::Position::Top,
    )
    .style(theme::inner_card_style)
    .into()
}

fn profile_label(app: &EnvBoxApp, id: uuid::Uuid) -> String {
    app.profiles
        .iter()
        .find(|profile| profile.id == id)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| format!("已删除的环境 · {}", crate::widgets::short_id(id)))
}
