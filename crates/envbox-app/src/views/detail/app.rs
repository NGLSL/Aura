use iced::widget::{button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::EnvBoxApp;
use crate::font;
use crate::icons::{icon, Icon};
use crate::message::{LaunchKind, Message, NamedId};
use crate::package::{Capability, InjectionSupport};
use crate::theme::*;
use crate::widgets::{
    app_icon_badge, badge, danger_btn, form_field, form_section, kv_row, primary_btn,
    secondary_btn, status_dot, toggle,
};
use envbox_core::ConsoleHost;

use super::{detail_editor_shell, detail_shell, section_title};

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
        .size(TEXT_CAPTION)
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
                    text("启动")
                        .size(TEXT_BODY)
                        .color(INK)
                        .font(font::name_font()),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([8, 14]))
            .style(primary_btn)
            .on_press(Message::AppRunId(id))
            .into()
        };

        if editing {
            run_button
        } else {
            row![
                button(text("编辑").size(TEXT_BODY).font(font::ui_font()))
                    .padding(Padding::from([8, 10]))
                    .style(secondary_btn)
                    .on_press(Message::AppEdit),
                run_button,
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        }
    } else {
        container(text("新建应用").size(TEXT_BODY).color(ACCENT_TEXT)).into()
    };

    let can_open_location = selected_app
        .map(|a| matches!(&a.launch, envbox_core::LaunchTarget::Executable { .. }))
        .unwrap_or(false);
    let open_loc_btn: Element<_> = if can_open_location {
        button(
            row![
                icon(Icon::ExternalLink, MUTED, 12.0),
                text("打开位置")
                    .size(TEXT_BODY)
                    .color(INK_2)
                    .font(font::ui_font()),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([8, 10]))
        .style(secondary_btn)
        .on_press(Message::AppOpenLocation)
        .into()
    } else {
        container(text("")).into()
    };

    let title_row = row![
        header_tile,
        column![
            text(app_name)
                .size(TEXT_TITLE)
                .color(INK)
                .font(font::name_font()),
            header_status,
        ]
        .spacing(2)
        .width(Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let action_row = row![
        open_loc_btn,
        iced::widget::Space::with_width(Fill),
        header_action,
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let mut header = column![title_row, action_row].spacing(8).width(Fill);
    if !editing {
        if let Some(application) = selected_app {
            header = header.push(run_selector(app, application));
        }
    }
    if running {
        header = header.push(
            button(text("查看运行实例").size(TEXT_BODY).font(font::ui_font()))
                .style(secondary_btn)
                .on_press(Message::InstanceFilter(selected)),
        );
    }

    if editing {
        let (content, actions) = app_editor(app);
        detail_editor_shell(column![header, content].spacing(12), actions)
    } else {
        detail_shell(column![header, app_overview(app, selected_app)].spacing(12))
    }
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
    .text_size(TEXT_BODY)
    .width(Fill)
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
                    .size(TEXT_SECTION)
                    .color(INK)
                    .font(font::name_font()),
                text("从左侧选择应用，或点击“添加应用”创建一个新的启动项。")
                    .size(TEXT_CAPTION)
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
        .unwrap_or_else(|| launch_kind_label(app.app_draft.kind).into());
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
    if selected_app.is_some() {
        content = content.push(
            button(
                row![
                    icon(Icon::ExternalLink, INK_2, 12.0),
                    text("创建桌面快捷方式")
                        .size(TEXT_BODY)
                        .font(font::ui_font()),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([8, 12]))
            .width(Fill)
            .style(secondary_btn)
            .on_press(Message::AppCreateDesktopShortcut(id)),
        );
    }
    if let Some(capability) = app.app_capabilities.get(&id) {
        if capability.injection != InjectionSupport::Supported {
            content = content.push(compatibility_notice(capability));
        }
    }
    if let Some(application) = selected_app.filter(|value| value.supports_chrome_data()) {
        content = content.push(kv_row(
            "浏览器数据",
            if application.chrome_independent_data {
                "按应用与环境独立保存"
            } else {
                "沿用启动参数 / Chrome 默认目录"
            },
        ));
        let choices: Vec<_> = app
            .profiles
            .iter()
            .map(|profile| (Some(profile.id), profile.name.clone()))
            .chain(std::iter::once((None, "本机 Host".into())))
            .flat_map(|(profile_id, label)| {
                envbox_core::DetectionSite::ALL
                    .into_iter()
                    .map(move |site| DetectionChoice {
                        label: format!("{label} · {site}"),
                        profile_id,
                        site,
                    })
            })
            .collect();
        content = content.push(
            text(format!(
                "默认环境：{}",
                app.profile_name(application.default_profile_id)
            ))
            .size(TEXT_CAPTION)
            .color(MUTED),
        );
        content = content.push(
            pick_list(choices, None::<DetectionChoice>, move |choice| {
                Message::AppDetect(id, choice.profile_id, choice.site)
            })
            .placeholder("选择环境并打开检测页")
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([7, 8]))
            .font(font::ui_font())
            .text_size(TEXT_BODY)
            .width(Fill),
        );
    }
    if selected_app.is_some_and(crate::package::chromium_renderer_sandbox_limited) {
        content = content.push(
            container(
                text("Chrome / Edge 新建浏览器进程时会按配置设置网页语言；复用已运行的浏览器需重启。网页沙箱中的原生系统信息仍可能显示本机值。")
                    .size(TEXT_CAPTION)
                    .color(ACCENT_TEXT)
                    .font(font::ui_font()),
            )
            .padding(Padding::from([10, 12]))
            .style(inner_card_style),
        );
    }
    container(content).width(Fill).into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetectionChoice {
    label: String,
    profile_id: Option<uuid::Uuid>,
    site: envbox_core::DetectionSite,
}
impl std::fmt::Display for DetectionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LaunchKindChoice(LaunchKind);

impl std::fmt::Display for LaunchKindChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(launch_kind_label(self.0))
    }
}

fn launch_kind_label(kind: LaunchKind) -> &'static str {
    match kind {
        LaunchKind::Command => "命令行",
        LaunchKind::Executable => "可执行文件",
        LaunchKind::Packaged => "Windows 应用包",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConsoleHostChoice(ConsoleHost);

impl std::fmt::Display for ConsoleHostChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            ConsoleHost::Direct => "直接启动",
            ConsoleHost::Cmd => "命令提示符",
            ConsoleHost::PowerShell => "Windows PowerShell",
            ConsoleHost::PowerShell7 => "PowerShell 7",
            ConsoleHost::WindowsTerminal => "Windows Terminal",
        })
    }
}

fn app_editor(app: &EnvBoxApp) -> (Element<'_, Message>, Element<'_, Message>) {
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
    let (target_label, target_placeholder) = match app.app_draft.kind {
        LaunchKind::Command => ("启动命令", "命令名称或完整命令行"),
        LaunchKind::Executable => ("程序路径", "可执行文件的完整路径"),
        LaunchKind::Packaged => ("应用标识", "Windows 应用的 AUMID"),
    };
    let mut basic = column![
        form_field(
            "应用名称",
            text_input("应用名称", &app.app_draft.name)
                .on_input(Message::AppName)
                .padding(Padding::from([8, 10]))
                .size(TEXT_BODY)
                .style(input_style)
                .font(font::ui_font()),
        ),
        form_field(
            "启动方式",
            pick_list(
                LaunchKind::ALL.map(LaunchKindChoice),
                Some(LaunchKindChoice(app.app_draft.kind)),
                |choice| Message::AppLaunchKind(choice.0),
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([8, 10]))
            .text_size(TEXT_BODY)
            .font(font::ui_font())
            .width(Fill),
        ),
        form_field(
            target_label,
            text_input(target_placeholder, &app.app_draft.path)
                .on_input(Message::AppPath)
                .padding(Padding::from([8, 10]))
                .size(TEXT_BODY)
                .style(input_style)
                .font(font::ui_font()),
        ),
    ]
    .spacing(FORM_SPACING);

    // A shell host only applies to commands. Keep an existing incompatible
    // selection visible so the user can correct it instead of losing it.
    if app.app_draft.kind == LaunchKind::Command
        || app.app_draft.console_host != ConsoleHost::Direct
    {
        basic = basic.push(form_field(
            "终端宿主",
            pick_list(
                ConsoleHost::ALL.map(ConsoleHostChoice),
                Some(ConsoleHostChoice(app.app_draft.console_host)),
                |choice| Message::AppConsoleHost(choice.0),
            )
            .style(pick_style)
            .menu_style(pick_menu)
            .padding(Padding::from([8, 10]))
            .text_size(TEXT_BODY)
            .font(font::ui_font())
            .width(Fill),
        ));
    }
    if app.app_draft.console_host != ConsoleHost::Direct
        && app.app_draft.kind != LaunchKind::Command
    {
        basic = basic.push(
            text("终端宿主仅适用于命令行，请切换启动方式或选择直接启动。")
                .size(TEXT_CAPTION)
                .color(DANGER_TEXT)
                .font(font::ui_font()),
        );
    } else if app.app_draft.console_host == ConsoleHost::WindowsTerminal {
        basic = basic.push(
            text("保存后，命令将在 Windows Terminal 中启动。")
                .size(TEXT_CAPTION)
                .color(MUTED)
                .font(font::ui_font()),
        );
    }

    let mut environment = column![form_field(
        "默认环境",
        pick_list(profile_labels, selected_profile, |n: NamedId| {
            Message::AppProfile(n.id)
        })
        .style(pick_style)
        .menu_style(pick_menu)
        .padding(Padding::from([8, 10]))
        .text_size(TEXT_BODY)
        .font(font::ui_font())
        .width(Fill),
    )]
    .spacing(FORM_SPACING);
    let chrome_supported = app.app_draft.kind == LaunchKind::Executable
        && std::path::Path::new(&app.app_draft.path)
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("chrome.exe"));
    if chrome_supported || app.app_draft.chrome_independent_data {
        environment = environment
            .push(
                row![
                    text("独立浏览器数据目录")
                        .size(TEXT_BODY)
                        .color(INK_2)
                        .font(font::ui_font())
                        .width(Fill),
                    toggle(
                        app.app_draft.chrome_independent_data,
                        Message::AppChromeIndependentDataChanged,
                    ),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .push(
                text("按应用和环境分别保存，不复制原有登录和书签。profile-directory 选择此目录内的子配置。")
                    .size(TEXT_CAPTION)
                    .color(MUTED)
                    .font(font::ui_font()),
            );
    }

    let advanced_toggle = button(
        row![
            text("更多启动选项")
                .size(TEXT_SECTION)
                .color(INK_2)
                .font(font::name_font())
                .width(Fill),
            icon(
                if app.app_advanced {
                    Icon::ChevronDown
                } else {
                    Icon::MoreHorizontal
                },
                MUTED,
                14.0,
            ),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding(0)
    .style(win_button)
    .on_press(Message::AppAdvancedToggle);
    let mut advanced = column![advanced_toggle].spacing(FORM_SPACING);
    if app.app_advanced {
        let workdir_input = row![
            text_input("留空则使用程序默认目录", &app.app_draft.work_dir)
                .on_input(Message::AppWorkDir)
                .padding(Padding::from([8, 10]))
                .size(TEXT_BODY)
                .style(input_style)
                .font(font::ui_font())
                .width(Fill),
            button(icon(Icon::Folder, MUTED, 14.0))
                .padding(Padding::from([10, 10]))
                .style(secondary_btn)
                .on_press(Message::AppBrowseWorkDir),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        advanced = advanced
            .push(form_field(
                "启动参数",
                text_input("例如 --profile dev", &app.app_draft.args)
                    .on_input(Message::AppArgs)
                    .padding(Padding::from([8, 10]))
                    .size(TEXT_BODY)
                    .style(input_style)
                    .font(font::ui_font()),
            ))
            .push(form_field("工作目录", workdir_input))
            .push(
                row![
                    text("子进程继续使用该环境")
                        .size(TEXT_BODY)
                        .color(INK_2)
                        .font(font::ui_font())
                        .width(Fill),
                    toggle(app.app_draft.inherit, Message::AppInherit),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .push(
                row![
                    text("记录环境读取审计")
                        .size(TEXT_BODY)
                        .color(INK_2)
                        .font(font::ui_font())
                        .width(Fill),
                    toggle(app.app_draft.audit, Message::AppAudit),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            );
    }
    let save = button(
        row![
            icon(Icon::Check, INK, 14.0),
            text("保存")
                .size(TEXT_BODY)
                .color(INK)
                .font(font::name_font()),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 16]))
    .width(Fill)
    .style(primary_btn)
    .on_press(Message::AppSave);
    let cancel = button(
        text("取消")
            .size(TEXT_BODY)
            .color(INK_2)
            .font(font::ui_font()),
    )
    .padding(Padding::from([8, 12]))
    .style(secondary_btn)
    .on_press(Message::AppEditCancel);
    let mut actions = row![save, cancel]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill);
    if app.app_draft.id.is_some() {
        actions = actions.push(
            button(
                text("删除")
                    .size(TEXT_BODY)
                    .color(DANGER_TEXT)
                    .font(font::ui_font()),
            )
            .padding(Padding::from([8, 12]))
            .style(danger_btn)
            .on_press(Message::AppDelete),
        );
    }

    let content = column![
        text("保存后用于下一次启动")
            .size(TEXT_CAPTION)
            .color(MUTED)
            .font(font::ui_font()),
        form_section("基本信息", basic),
        form_section("环境与数据", environment),
        container(advanced)
            .padding(14)
            .width(Fill)
            .style(panel_card_style),
    ]
    .spacing(FORM_SPACING);
    (content.into(), actions.into())
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
                .size(TEXT_CAPTION)
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
