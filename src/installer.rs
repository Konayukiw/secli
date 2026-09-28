use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::PathBuf;
use windows::core::w;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const APP_DIR_NAME: &str = "SoftEther-CLI";
const EXE_NAME: &str = "secli.exe";

pub fn get_install_dir() -> Result<PathBuf> {
    let local_app_data = env::var("LOCALAPPDATA")
        .context("LOCALAPPDATA environment variable is not defined")?;
    let path = PathBuf::from(local_app_data)
        .join("Programs")
        .join(APP_DIR_NAME);
    Ok(path)
}

pub fn install_self() -> Result<()> {
    let current_exe = env::current_exe().context("Could not determine current executable path")?;
    let install_dir = get_install_dir()?;

    println!("==> Installing SoftEther-CLI...");
    println!("Target directory: {}", install_dir.display());

    fs::create_dir_all(&install_dir).context("Failed to create target installation directory")?;

    let dest_exe = install_dir.join(EXE_NAME);

    if current_exe != dest_exe {
        if dest_exe.exists() {
            let backup = install_dir.join(format!("{}.old", EXE_NAME));
            let _ = fs::remove_file(&backup);
            let _ = fs::rename(&dest_exe, &backup);
        }
        fs::copy(&current_exe, &dest_exe).context("Failed to copy binary to install directory")?;
        println!("Copied {} -> {}", current_exe.display(), dest_exe.display());
    } else {
        println!("Binary is already at target path: {}", dest_exe.display());
    }

    add_dir_to_user_path(&install_dir)?;

    println!("==> Installation completed successfully!");
    println!("You can now run 'secli' from any new terminal window or command prompt.");

    Ok(())
}

fn add_dir_to_user_path(dir: &PathBuf) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env_key, _) = hkcu
        .create_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .context("Failed to open or create HKCU\\Environment registry key")?;

    let current_path: String = env_key.get_value("Path").unwrap_or_default();
    let dir_str = dir.to_string_lossy().to_string();

    let parts: Vec<&str> = current_path
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let already_present = parts
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&dir_str));

    if already_present {
        println!("Path is already registered in User PATH environment variable.");
    } else {
        let new_path = if current_path.trim().is_empty() {
            dir_str.clone()
        } else {
            format!("{};{}", current_path.trim_end_matches(';'), dir_str)
        };

        env_key
            .set_value("Path", &new_path)
            .context("Failed to update User PATH in registry")?;
        println!("Successfully added '{}' to User PATH in registry.", dir_str);

        broadcast_env_change();
    }

    Ok(())
}

fn broadcast_env_change() {
    unsafe {
        let mut result = 0;
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            Some(&mut result),
        );
    }
}
