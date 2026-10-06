use iced::widget::{button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{LaunchKind, Message, NamedId};
use crate::package::{Capability, InjectionSupport};
use crate::theme::*;
use crate::widgets::{
    app_icon_badge, badge, danger_btn, field_label, form_row, kv_row, primary_btn, secondary_btn,
    status_dot, toggle,
};
use envbox_core::ConsoleHost;

use super::{detail_shell, section_title};

pub(super) fn view(app: &EnvBoxApp) -> Element<'_, Message> {
    let selected = app.app_draft.id;
    let editing = app.app_edit_mode || selected.is_none();
    let running_count = selected.map(|id| app.running_count(id)).unwrap_or(0);
    let running = running_count > 0;
    let selected_app = selected.and_then(|id| app.applications.iter().find(|a| a.id == id));

    let app_name = if app.app_draft.name.is_empty() {
        "未选择应用"
    } else {
        app.app_draft.name.as_str()
    };

    let header_tile: Element<_> = selected
        .and_then(|id| app.app_icons.get(&id))
        .map(|png| {
            container(
                iced::widget::image(iced::widget::image::Handle::from_path(png.clone()))
                    .width(Length::Fixed(36.0))
                    .height(Length::Fixed(36.0)),
            )
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .style(|_| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.05, 0.07, 0.10,
                ))),
                border: iced::Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..Default::default()
            })
            .center_x(40.0)
            .center_y(40.0)
            .into()
        })
        .unwrap_or_else(|| app_icon_badge(&app.app_draft.name, 40.0));

    let header_status = row![
        status_dot(if running { SUCCESS } else { FAINT }),
        text(if running {
            format!("运行中 · {running_count} 个实例")
        } else {
            "未运行".to_string()
        })
        .size(12)
        .color(if running { SUCCESS } else { MUTED })
        .font(font::ui_font()),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let header_action: Element<_> = if let Some(id) = selected {
        let run_button: Element<_> = {
            button(
                row![
                    icon(Icon::Play, INK, 11.0),
                    text("启动").size(12).color(INK).font(font::name_font()),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([6, 14]))
            .style(primary_btn)
            .on_press(Message::AppRunId(id))
            .into()
        };

        if editing {
            row![
                button(text("取消").size(12).font(font::ui_font()))
                    .padding(Padding::from([6, 10]))
                    .style(secondary_btn)
                    .on_press(Message::AppEditCancel),
                run_button,
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                button(text("编辑").size(12).font(font::ui_font()))
                    .padding(Padding::from([6, 10]))
                    .style(secondary_btn)
                    .on_press(Message::AppEdit),
                run_button,
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        }
    } else {
        container(text("新建应用").size(12).color(ACCENT_TEXT)).into()
    };

    let can_open_location = selected_app
        .map(|a| matches!(&a.launch, envbox_core::LaunchTarget::Executable { .. }))
        .unwrap_or(false);
    let open_loc_btn: Element<_> = if can_open_location {
        button(
            row![
                icon(Icon::ExternalLink, MUTED, 12.0),
                text("打开位置").size(11).color(INK_2).font(font::ui_font()),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([6, 9]))
        .style(secondary_btn)
        .on_press(Message::AppOpenLocation)
        .into()
    } else {
        container(text("")).into()
    };

    let title_row = row![
        header_tile,
        column![
            text(app_name).size(17).color(INK).font(font::name_font()),
            header_status,
        ]
        .spacing(2)
        .width(Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let mut action_row = row![open_loc_btn, header_action]
        .spacing(6)
        .align_y(Alignment::Center);
    if !editing {
        if let Some(application) = selected_app {
            action_row = action_row.push(run_selector(app, application));
        }
    }

    let mut header = column![title_row, action_row].spacing(8).width(Fill);
    if running {
        header = header.push(
            button(text("查看运行实例").size(12).font(font::ui_font()))
                .style(secondary_btn)
                .on_press(Message::InstanceFilter(selected)),
        );
    }

    let content = if editing {
        app_editor(app)
    } else {
        app_overview(app, selected_app)
    };

    detail_shell(column![header, content].spacing(12))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunChoice {
    label: String,
    profile_id: Option<uuid::Uuid>,
}

impl std::fmt::Display for RunChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn run_selector<'a>(
    app: &'a EnvBoxApp,
    application: &'a envbox_core::Application,
) -> Element<'a, Message> {
    let id = application.id;
    let unsupported = app
        .app_capabilities
        .get(&id)
        .is_some_and(|capability| capability.injection == InjectionSupport::Unsupported);
    let mut choices: Vec<_> = app
        .profiles
        .iter()
        .filter(|profile| !unsupported && profile.id != application.default_profile_id)
        .map(|profile| RunChoice {
            label: if app
                .profiles
                .iter()
                .filter(|other| other.name == profile.name)
                .count()
                > 1
            {
                format!("{} ({})", profile.name, &profile.id.to_string()[..8])
            } else {
                profile.name.clone()
            },
            profile_id: Some(profile.id),
        })
        .collect();
    choices.push(RunChoice {
        label: "在本机直接运行".into(),
        profile_id: None,
    });
    pick_list(choices, None::<RunChoice>, move |choice| {
        Message::AppRunWithId(id, choice.profile_id)
    })
    .placeholder("其他方式")
    .style(pick_style)
    .menu_style(pick_menu)
    .padding(Padding::from([5, 8]))
    .font(font::ui_font())
    .text_size(12)
    .into()
}

fn app_overview<'a>(
    app: &'a EnvBoxApp,
    selected_app: Option<&'a envbox_core::Application>,
) -> Element<'a, Message> {
    let Some(id) = app.app_draft.id else {
        return container(
            column![
                text("还没有选择应用")
                    .size(15)
                    .color(INK)
                    .font(font::name_font()),
                text("从左侧选择应用，或点击“添加应用”创建一个新的启动项。")
                    .size(12)
                    .color(MUTED)
                    .font(font::ui_font()),
            ]
            .spacing(8),
        )
        .padding(14)
        .width(Fill)
        .style(inner_card_style)
        .into();
    };

    let profile_name = app.profile_name(app.app_draft.profile_id);
    let launch = selected_app
        .map(|a| launch_target_label(&a.launch))
        .unwrap_or_else(|| app.app_draft.kind.to_string());
    let mut settings = column![
        kv_row("默认环境", &profile_name),
        kv_row("启动方式", &launch),
    ]
    .spacing(7);
    if let Some(selected_app) = selected_app {
        if let Some((label, target)) = application_target(selected_app) {
            settings = settings.push(kv_row(label, &target));
        }
    }

    let mut content = column![
        section_title("启动设置", "运行时使用已保存的设置"),
        container(settings.padding(10))
            .width(Fill)
            .style(inner_card_style),
    ]
    .spacing(10);
    if let Some(capability) = app.app_capabilities.get(&id) {
        if capability.injection != InjectionSupport::Supported {
            content = content.push(compatibility_notice(capability));
        }
    }
    if selected_app.is_some_and(crate::package::chromium_renderer_sandbox_limited) {
        content = content.push(
            container(
                text("Chrome / Edge 新建浏览器进程时会按配置设置网页语言；复用已运行的浏览器需重启。网页沙箱中的原生系统信息仍可能显示本机值。")
                    .size(12)
                    .color(ACCENT_TEXT)
                    .font(font::ui_font()),
            )
            .padding(Padding::from([10, 12]))
            .style(inner_card_style),
        );
    }
    container(content).width(Fill).into()
}

fn app_editor(app: &EnvBoxApp) -> Element<'_, Message> {
    let profile_labels: Vec<NamedId> = app
        .profiles
        .iter()
        .map(|p| NamedId {
            name: p.name.clone(),
            id: p.id,
        })
        .collect();
    let selected_profile = profile_labels
        .iter()
        .find(|n| n.id == app.app_draft.profile_id)
        .cloned();

    let console_hint: Element<'_, Message> = if app.app_draft.console_host != ConsoleHost::Direct
        && app.app_draft.kind != LaunchKind::Command
    {
        container(
            text(format!(
                "{} 仅适用于 Command 目标；请先将启动类型改为 Command。",
                app.app_draft.console_host
            ))
            .size(11)
            .color(DANGER_TEXT)
            .font(font::ui_font()),
        )
        .padding(Padding::from([4, 8]))
        .style(inner_card_style)
        .into()
    } else if app.app_draft.console_host == ConsoleHost::WindowsTerminal {
        container(
            text("Windows Terminal 将在保存后用于命令目标的启动。")
                .size(11)
                .color(MUTED)
                .font(font::ui_font()),
        )
        .padding(Padding::from([4, 8]))
        .into()
    } else {
        container(text("")).into()
    };

    let basic = column![
        field_label("基本信息"),
        form_row(
            "应用名称",
            text_input("应用名称", &app.app_draft.name)
                .on_input(Message::AppName)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font()),
        ),
        form_row(
            "启动方式",
            pick_list(
                LaunchKind::ALL,
                Some(app.app_draft.kind),
                Message::AppLaunchKind,
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([5, 8]))
            .font(font::ui_font()),
        ),
        form_row(
            "命令 / 路径",
            text_input("可执行文件路径或命令行", &app.app_draft.path)
                .on_input(Message::AppPath)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font()),
        ),
        form_row(
            "终端宿主",
            pick_list(
                ConsoleHost::ALL,
                Some(app.app_draft.console_host),
                Message::AppConsoleHost,
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([5, 8]))
            .font(font::ui_font()),
        ),
        console_hint,
        form_row(
            "默认环境",
            pick_list(profile_labels, selected_profile, |n: NamedId| {
                Message::AppProfile(n.id)
            })
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([5, 8]))
            .font(font::ui_font()),
        ),
    ]
    .spacing(8);

    let advanced_toggle = button(
        row![
            icon(
                if app.app_advanced {
                    Icon::ChevronDown
                } else {
                    Icon::MoreHorizontal
                },
                ACCENT_LINE,
                11.0,
            ),
            text("高级启动选项")
                .size(12)
                .color(INK_2)
                .font(font::name_font()),
            text("参数、工作目录和进程行为")
                .size(11)
                .color(MUTED)
                .font(font::ui_font()),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding(Padding::from([8, 10]))
    .style(secondary_btn)
    .on_press(Message::AppAdvancedToggle);

    let advanced: Element<_> = if app.app_advanced {
        let workdir_input = row![
            text_input("留空则使用程序默认目录", &app.app_draft.work_dir)
                .on_input(Message::AppWorkDir)
                .padding(Padding::from([5, 8]))
                .style(input_style)
                .font(font::ui_font())
                .width(Fill),
            button(icon(Icon::Folder, MUTED, 13.0))
                .padding(Padding::from([5, 8]))
                .style(secondary_btn)
                .on_press(Message::AppBrowseWorkDir),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        container(
            column![
                form_row(
                    "启动参数",
                    text_input("可选命令行参数，例如 --profile dev", &app.app_draft.args)
                        .on_input(Message::AppArgs)
                        .padding(Padding::from([5, 8]))
                        .style(input_style)
                        .font(font::ui_font()),
                ),
                form_row("工作目录", workdir_input),
                row![
                    text("子进程继续使用该环境")
                        .size(12)
                        .color(INK_2)
                        .font(font::ui_font())
                        .width(Fill),
                    toggle(app.app_draft.inherit, Message::AppInherit),
                ]
                .align_y(Alignment::Center),
                row![
                    text("记录环境读取审计")
                        .size(12)
                        .color(INK_2)
                        .font(font::ui_font())
                        .width(Fill),
                    toggle(app.app_draft.audit, Message::AppAudit),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .padding(10),
        )
        .width(Fill)
        .style(inner_card_style)
        .into()
    } else {
        container(text("")).into()
    };

    let save = button(
        row![
            icon(Icon::Check, INK, 12.0),
            text("保存").size(13).color(INK).font(font::name_font()),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 16]))
    .width(Fill)
    .style(primary_btn)
    .on_press(Message::AppSave);
    let cancel = button(text("取消").size(13).color(INK_2).font(font::ui_font()))
        .padding(Padding::from([8, 13]))
        .style(secondary_btn)
        .on_press(Message::AppEditCancel);
    let delete: Element<_> = if app.app_draft.id.is_some() {
        button(
            text("删除应用")
                .size(11)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        )
        .padding(Padding::from([6, 9]))
        .style(danger_btn)
        .on_press(Message::AppDelete)
        .into()
    } else {
        container(text("")).into()
    };

    column![
        section_title("编辑应用", "只在保存后应用到下一次启动"),
        basic,
        advanced_toggle,
        advanced,
        row![save, cancel, delete]
            .spacing(8)
            .align_y(Alignment::Center),
    ]
    .spacing(10)
    .into()
}
fn application_target(app: &envbox_core::Application) -> Option<(&'static str, String)> {
    match &app.launch {
        envbox_core::LaunchTarget::Command { command } => Some(("命令", command.clone())),
        envbox_core::LaunchTarget::Executable { path } => {
            Some(("程序路径", path.display().to_string()))
        }
        envbox_core::LaunchTarget::Packaged { .. } => None,
    }
}

fn launch_target_label(target: &envbox_core::LaunchTarget) -> String {
    match target {
        envbox_core::LaunchTarget::Command { .. } => "命令行".into(),
        envbox_core::LaunchTarget::Executable { .. } => "可执行文件".into(),
        envbox_core::LaunchTarget::Packaged { .. } => "Windows 应用包".into(),
    }
}

fn compatibility_notice(cap: &Capability) -> Element<'static, Message> {
    let (bg, fg) = match cap.injection {
        InjectionSupport::Supported => (SUCCESS_BG, SUCCESS_TEXT),
        InjectionSupport::Delayed => (ACCENT_BG, ACCENT_TEXT),
        InjectionSupport::Unsupported => (DANGER_BG, DANGER_TEXT),
    };
    container(
        column![
            badge(cap.badge(), bg, fg),
            text(cap.user_explanation())
                .size(11)
                .color(INK_2)
                .font(font::ui_font()),
        ]
        .spacing(6)
        .padding(10),
    )
    .width(Fill)
    .style(inner_card_style)
    .into()
}
