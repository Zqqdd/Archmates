//! What your profile says about a Windows system, as a Linux user would expect to read it:
//! edition, build, uptime, terminal, shell, and your account picture.

use std::path::PathBuf;

use super::reg;
use crate::platform::SystemInfo;

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// "Windows 11 Pro 24H2" (the registry still says "Windows 10" on Windows 11).
fn edition() -> String {
    let name = reg::get_string(reg::LOCAL_MACHINE, CURRENT_VERSION, "ProductName").unwrap_or_else(|| "Windows".into());
    let name = if build() >= 22000 { name.replace("Windows 10", "Windows 11") } else { name };
    match reg::get_string(reg::LOCAL_MACHINE, CURRENT_VERSION, "DisplayVersion") {
        Some(v) if !v.is_empty() => format!("{name} {v}"),
        _ => name,
    }
}

fn build() -> u32 {
    reg::get_string(reg::LOCAL_MACHINE, CURRENT_VERSION, "CurrentBuild").and_then(|b| b.parse().ok()).unwrap_or(0)
}

/// "NT 10.0.26100.2605"
fn kernel() -> String {
    let ubr = reg::get_dword(reg::LOCAL_MACHINE, CURRENT_VERSION, "UBR").unwrap_or(0);
    format!("NT 10.0.{}.{ubr}", build())
}

fn uptime_s() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() / 1000 }
}

fn local_app_data(rel: &str) -> bool {
    std::env::var_os("LOCALAPPDATA").is_some_and(|d| std::path::Path::new(&d).join(rel).exists())
}

fn terminal() -> String {
    if std::env::var_os("WT_SESSION").is_some() || local_app_data(r"Microsoft\WindowsApps\wt.exe") {
        "Windows Terminal".into()
    } else {
        "conhost".into()
    }
}

fn shell() -> String {
    let pwsh7 = std::env::var_os("ProgramFiles")
        .is_some_and(|d| std::path::Path::new(&d).join(r"PowerShell\7\pwsh.exe").exists());
    if pwsh7 { "pwsh".into() } else { "PowerShell".into() }
}

pub fn system_info(_user: &str) -> SystemInfo {
    SystemInfo {
        host: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "windows".into()),
        distro: edition(),
        kernel: kernel(),
        wm: "Explorer".into(),
        terminal: terminal(),
        shell: shell(),
        uptime_s: uptime_s(),
    }
}

/// The account picture set in Windows settings (largest size available).
fn account_picture() -> Option<PathBuf> {
    let out = super::command("whoami").args(["/user", "/fo", "csv", "/nh"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let sid = text.trim().rsplit(',').next()?.trim_matches('"').to_string();
    if !sid.starts_with("S-") {
        return None;
    }
    let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\AccountPicture\Users\{sid}");
    ["Image448", "Image240", "Image208", "Image192"]
        .iter()
        .find_map(|v| reg::get_string(reg::LOCAL_MACHINE, &key, v))
        .map(PathBuf::from)
}

/// System places a profile picture can be: your Windows account picture.
pub fn avatar_candidates(_user: &str) -> Vec<PathBuf> {
    account_picture().into_iter().collect()
}
