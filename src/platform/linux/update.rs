//! Installing a release on Linux: unpack the checked archive and run the installer it contains.

use anyhow::{Result, anyhow};

/// True when this copy is managed by pacman (installed under /usr): pacman must update it.
pub fn managed_by_pacman() -> bool {
    std::env::current_exe().map(|e| e.starts_with("/usr")).unwrap_or(false)
}

/// How to update this copy, for the "update available" notification.
pub fn update_hint() -> String {
    if managed_by_pacman() { "sudo pacman -Syu".into() } else { "archmates update --install".into() }
}

/// Unpacks release `version` (already downloaded and checked) in a private folder and runs its
/// installer. Returns once installed.
pub async fn install_release(version: &str) -> Result<()> {
    let name = format!("archmates-{version}-x86_64-linux");
    let archive = crate::common::update::download_checked(version, &format!("{name}.tar.gz")).await?;

    let work = crate::common::config::Paths::detect().cache.join("update");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let tarball = work.join(format!("{name}.tar.gz"));
    std::fs::write(&tarball, &archive)?;
    let listing = std::process::Command::new("tar").arg("-tzf").arg(&tarball).output()?;
    let entries = String::from_utf8_lossy(&listing.stdout);
    if !listing.status.success()
        || entries.lines().any(|e| !e.starts_with(&format!("{name}/")) || e.split('/').any(|part| part == ".."))
    {
        return Err(anyhow!("unexpected archive layout (nothing installed)"));
    }
    let status = std::process::Command::new("tar")
        .args(["-xzf"])
        .arg(&tarball)
        .args(["--no-same-owner", "-C"])
        .arg(&work)
        .status()?;
    if !status.success() {
        return Err(anyhow!("could not unpack the archive"));
    }
    println!("→ Running the installer");
    let dir = work.join(&name);
    let status = std::process::Command::new(dir.join("installer.sh")).current_dir(&dir).status()?;
    let _ = std::fs::remove_dir_all(&work);
    if !status.success() {
        return Err(anyhow!("the installer failed ({status})"));
    }
    Ok(())
}

/// True when the program file was replaced on disk (after an update):
/// the running service should restart to use the new version.
pub fn binary_replaced() -> bool {
    std::fs::read_link("/proc/self/exe").map(|p| p.to_string_lossy().ends_with(" (deleted)")).unwrap_or(false)
}
