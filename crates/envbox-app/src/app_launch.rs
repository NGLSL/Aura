//! Headless startup for desktop shortcuts.
//!
//! A shortcut stores an Application UUID (and, when needed, the configuration
//! root) rather than copying the application's launch details into the link.
//! This module resolves that UUID from the current persisted documents and
//! reuses the same `InstanceManager` path as the GUI.  It deliberately does
//! not initialize Iced, the tray, or the GUI singleton.

use envbox_core::{Application, LaunchTarget};
use envbox_launcher::{InstanceManager, RunTarget};
use envbox_storage::{validate_application, ConfigStore};
use std::path::PathBuf;
use uuid::Uuid;

/// Handle the optional desktop-shortcut startup mode.
///
/// `None` means that no `--launch-app` flag was supplied and the normal GUI
/// startup should continue.  Once the flag is present, every parse, storage,
/// capability, and launch failure is returned as `Some(Err(_))` so the caller
/// can show a visible startup error and exit without constructing the GUI.
pub fn handle_startup() -> Option<Result<(), String>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(None) => return None,
        Ok(Some(options)) => options,
        Err(error) => return Some(Err(error)),
    };

    Some(launch_application(options))
}

#[derive(Debug)]
struct StartupOptions {
    application_id: Uuid,
    config_root: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Option<StartupOptions>, String> {
    // Preserve the normal GUI argument contract unless this explicit startup
    // flag selects shortcut mode.  In particular, a stray `--config-root`
    // alone must not prevent the regular window from opening.
    if !args
        .iter()
        .any(|argument| argument == "--launch-app" || argument.starts_with("--launch-app="))
    {
        return Ok(None);
    }

    let mut application_id = None;
    let mut config_root = None;
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];
        if argument == "--launch-app" {
            if application_id.is_some() {
                return Err("--launch-app 只能指定一次".into());
            }
            let value = args
                .get(index + 1)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "--launch-app 需要 Application UUID".to_string())?;
            application_id = Some(
                Uuid::parse_str(value)
                    .map_err(|_| format!("--launch-app 的 UUID 无效：{value:?}"))?,
            );
            index += 2;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--launch-app=") {
            if application_id.is_some() {
                return Err("--launch-app 只能指定一次".into());
            }
            if value.is_empty() {
                return Err("--launch-app 需要 Application UUID".into());
            }
            application_id = Some(
                Uuid::parse_str(value)
                    .map_err(|_| format!("--launch-app 的 UUID 无效：{value:?}"))?,
            );
            index += 1;
            continue;
        }
        if argument == "--config-root" {
            if config_root.is_some() {
                return Err("--config-root 只能指定一次".into());
            }
            let value = args
                .get(index + 1)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "--config-root 需要绝对路径".to_string())?;
            config_root = Some(parse_config_root(value)?);
            index += 2;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--config-root=") {
            if config_root.is_some() {
                return Err("--config-root 只能指定一次".into());
            }
            config_root = Some(parse_config_root(value)?);
            index += 1;
            continue;
        }

        return Err(format!("快捷启动参数无法识别：{argument}"));
    }

    let Some(application_id) = application_id else {
        return Ok(None);
    };
    Ok(Some(StartupOptions {
        application_id,
        config_root,
    }))
}

fn parse_config_root(value: &str) -> Result<PathBuf, String> {
    if value.trim().is_empty() {
        return Err("--config-root 需要绝对路径".into());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("--config-root 必须是绝对路径：{}", path.display()));
    }
    Ok(path)
}

fn launch_application(options: StartupOptions) -> Result<(), String> {
    // The explicit root must also reach the Windows Terminal bridge.  The
    // bridge receives the root as a command-line value, but the launcher uses
    // this process variable as its source when constructing that handoff.
    if let Some(root) = &options.config_root {
        std::env::set_var("ENVBOX_CONFIG_ROOT", root);
    }
    let root = options
        .config_root
        .unwrap_or_else(ConfigStore::default_root);
    let store = ConfigStore::new(root);

    let application = store
        .load_applications()
        .map_err(|error| format!("应用配置读取失败：{error}"))?
        .applications
        .into_iter()
        .find(|application| application.id == options.application_id)
        .ok_or_else(|| format!("应用不存在：{}", options.application_id))?;
    validate_application(&application).map_err(|error| format!("已保存的应用配置无效：{error}"))?;

    let profile = store
        .load_profiles()
        .map_err(|error| format!("环境配置读取失败：{error}"))?
        .profiles
        .into_iter()
        .find(|profile| profile.id == application.default_profile_id)
        .ok_or_else(|| {
            format!(
                "应用「{}」绑定的环境配置不存在：{}",
                application.name, application.default_profile_id
            )
        })?;

    reject_unsupported_capability(&application)?;

    let is_windows_terminal = application.console_host == envbox_core::ConsoleHost::WindowsTerminal;
    let mut instances = InstanceManager::new();
    let instance_id = instances
        .run(&application, RunTarget::Profile(profile))
        .map_err(|error| format!("启动应用失败：{error}"))?;

    // Normal Profile sessions have completed the independent session-host
    // handshake when `run` returns, so the launcher can exit immediately.
    // Windows Terminal first needs the hidden terminal-run process to open
    // the named Job and publish the root marker; keep the Job handle alive
    // until that real handoff is observed.
    if is_windows_terminal {
        instances
            .wait_for_handoff(instance_id)
            .map_err(|error| format!("Windows Terminal 交接失败：{error}"))?;
    }
    Ok(())
}

fn reject_unsupported_capability(application: &Application) -> Result<(), String> {
    let target = match &application.launch {
        LaunchTarget::Command { command } => command.as_str(),
        LaunchTarget::Executable { path } => path.to_str().unwrap_or_default(),
        LaunchTarget::Packaged { aumid, .. } => aumid.as_str(),
    };
    let capability = crate::package::classify_target(
        target,
        &envbox_launcher::format_args(&application.arguments),
    );
    if capability.injection == crate::package::InjectionSupport::Unsupported {
        return Err(format!(
            "应用「{}」无法使用环境配置启动。{}",
            application.name,
            capability.user_explanation()
        ));
    }
    Ok(())
}
