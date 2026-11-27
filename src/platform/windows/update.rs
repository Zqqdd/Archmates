//! Installing a release on Windows: the checked zip's programs replace the installed ones
//! in %LOCALAPPDATA%\Programs\Winmates (the running ones keep working until restarted).

use anyhow::{Context, Result, anyhow};

/// The programs shipped in the Windows archive: the app, WinmatesUpdater, and the service +
/// command line (same as on Linux).
pub const PROGRAMS: [&str; 3] = [super::install::APP, super::install::UPDATER, super::install::CLI];

/// Never managed by pacman on Windows.
pub fn managed_by_pacman() -> bool {
    false
}

pub fn update_hint() -> String {
    "restart Winmates (it updates itself when it starts)".into()
}

/// Windows can't tell a running program it was replaced: Winmates restarts the service itself.
pub fn binary_replaced() -> bool {
    false
}

pub async fn install_release(version: &str) -> Result<()> {
    let name = format!("winmates-{version}-x86_64-windows");
    let archive = crate::common::update::download_checked(version, &format!("{name}.zip")).await?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).context("unreadable archive")?;
    let mut files = Vec::new();
    for wanted in PROGRAMS {
        // `<name>/winmates.exe`, whichever separator the archiver used. Only the bytes are
        // taken: where they go is decided here, never by the archive.
        let index = (0..zip.len())
            .find(|&i| zip.name_for_index(i).is_some_and(|n| n.replace('\\', "/") == format!("{name}/{wanted}")))
            .ok_or_else(|| anyhow!("{wanted} is missing from the archive (nothing installed)"))?;
        let mut entry = zip.by_index(index)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)?;
        files.push((wanted, bytes));
    }
    let dir = super::install_dir();
    std::fs::create_dir_all(&dir)?;
    for (file, bytes) in files {
        replace_program(&dir.join(file), &bytes)?;
    }
    println!("→ Installed in {}", dir.display());
    Ok(())
}

/// Windows can't overwrite a running program, but it can rename it: the old one is moved
/// aside (deleted at the next start, see `cleanup_old_programs`) and the new one takes its name.
pub fn replace_program(target: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if target.exists() {
        let aside = target.with_extension(format!("old-{}", crate::common::time::now_ms()));
        std::fs::rename(target, &aside).with_context(|| format!("cannot replace {}", target.display()))?;
    }
    let tmp = target.with_extension("new");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// Removes programs left aside by an update (those still running stay until next time).
pub fn cleanup_old_programs() {
    let Ok(entries) = std::fs::read_dir(super::install_dir()) else { return };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().contains(".old-") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
