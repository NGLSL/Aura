//! Desktop links refer to a saved Application, not a copy of its launch settings.

use std::path::{Path, PathBuf};

use envbox_core::{Application, LaunchTarget};
use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::{FOLDERID_Desktop, IShellLinkW, SHGetKnownFolderPath, ShellLink};

/// Called on a background thread so Shell COM does not change the UI apartment.
pub fn create_desktop(
    app: &Application,
    config_root: &Path,
    icon_png: Option<&Path>,
) -> Result<PathBuf, String> {
    envbox_storage::validate_application(app).map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let config_root = std::path::absolute(config_root).map_err(|error| error.to_string())?;
    let profiles = envbox_storage::ConfigStore::new(config_root.clone())
        .load_profiles()
        .map_err(|error| format!("读取环境配置失败：{error}"))?;
    let profile = profiles
        .profiles
        .iter()
        .find(|profile| profile.id == app.default_profile_id)
        .ok_or_else(|| format!("应用绑定的环境配置不存在：{}", app.default_profile_id))?;
    let root_argument = config_root
        .to_str()
        .ok_or("配置目录包含无法用于启动参数的字符")?;
    let arguments = envbox_launcher::format_args(&[
        "--launch-app".into(),
        app.id.to_string(),
        "--config-root".into(),
        root_argument.into(),
    ]);

    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| format!("初始化桌面快捷方式失败：{error}"))?;
    let _apartment = Apartment;
    let desktop = desktop_directory()?;
    let stem = format!(
        "{} - {}",
        shortcut_name(&app.name),
        shortcut_name(&profile.name)
    );
    let mut number = 1;
    let destination = loop {
        let name = if number == 1 {
            format!("{stem}.lnk")
        } else {
            format!("{stem} ({number}).lnk")
        };
        let candidate = desktop.join(name);
        if !candidate.exists() || matches_link(&candidate, &executable, &arguments) {
            break candidate;
        }
        number += 1;
    };

    let icon = shortcut_icon(app, icon_png, &config_root).unwrap_or_else(|| executable.clone());
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| error.to_string())?;
        link.SetPath(PCWSTR(wide(&executable).as_ptr()))
            .map_err(|error| error.to_string())?;
        link.SetArguments(PCWSTR(wide(Path::new(&arguments)).as_ptr()))
            .map_err(|error| error.to_string())?;
        if let Some(directory) = executable.parent() {
            link.SetWorkingDirectory(PCWSTR(wide(directory).as_ptr()))
                .map_err(|error| error.to_string())?;
        }
        link.SetIconLocation(PCWSTR(wide(&icon).as_ptr()), 0)
            .map_err(|error| error.to_string())?;
        let persist: IPersistFile = link.cast().map_err(|error| error.to_string())?;
        persist
            .Save(PCWSTR(wide(&destination).as_ptr()), true)
            .map_err(|error| format!("保存桌面快捷方式失败：{error}"))?;
    }
    Ok(destination)
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn desktop_directory() -> Result<PathBuf, String> {
    unsafe {
        let pointer = SHGetKnownFolderPath(&FOLDERID_Desktop, Default::default(), None)
            .map_err(|error| format!("读取桌面目录失败：{error}"))?;
        let path = pointer.to_string();
        CoTaskMemFree(Some(pointer.0.cast()));
        let path = PathBuf::from(path.map_err(|error| error.to_string())?);
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(path)
    }
}

fn shortcut_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            }
        })
        .take(80)
        .collect();
    let name = name.trim().trim_end_matches(['.', ' ']);
    if name.is_empty() {
        "应用".into()
    } else {
        name.into()
    }
}

/// Repeated creation refreshes our own link and leaves other desktop files alone.
fn matches_link(path: &Path, executable: &Path, arguments: &str) -> bool {
    unsafe {
        let Ok(link): Result<IShellLinkW, _> =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
        else {
            return false;
        };
        let Ok(persist) = link.cast::<IPersistFile>() else {
            return false;
        };
        if persist
            .Load(PCWSTR(wide(path).as_ptr()), STGM_READ)
            .is_err()
        {
            return false;
        }
        let mut target = vec![0u16; 32_768];
        let mut args = vec![0u16; 32_768];
        if link.GetPath(&mut target, std::ptr::null_mut(), 0).is_err()
            || link.GetArguments(&mut args).is_err()
        {
            return false;
        }
        let target_length = target
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(target.len());
        let args_length = args
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(args.len());
        let expected = wide(executable);
        target[..target_length] == expected[..expected.len() - 1]
            && String::from_utf16_lossy(&args[..args_length]) == arguments
    }
}

fn shortcut_icon(app: &Application, png: Option<&Path>, config_root: &Path) -> Option<PathBuf> {
    if let LaunchTarget::Executable { path } = &app.launch {
        if path.is_file() {
            return Some(path.clone());
        }
    }
    // The GUI normally supplies its asynchronously cached PNG. A desktop
    // shortcut can be created before that task finishes, though, so resolve
    // the same application icon on demand (notably for packaged AUMIDs).
    let cached = crate::app_icon::cache_application_icon(&config_root.join("icons"), app)
        .or_else(|| png.filter(|path| path.is_file()).map(Path::to_path_buf));
    let image = image::open(cached?).ok()?;
    let directory = config_root.join("icons").join("shortcuts");
    std::fs::create_dir_all(&directory).ok()?;
    let icon = directory.join(format!("{}.ico", app.id));
    image
        .save_with_format(&icon, image::ImageFormat::Ico)
        .ok()?;
    Some(icon)
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
