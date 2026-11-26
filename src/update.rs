//! Updates.
//!
//! - The service only *checks* GitHub once a day and notifies: it never installs anything by itself.
//! - `archmates update --install`, run by the user, downloads the latest release archive from
//!   GitHub over HTTPS, checks its SHA-256 checksum, and runs the installer it contains.
//! - Windows: Winmates does that by itself each time it starts (same download, same
//!   checksum check), then puts the new programs in place of the old ones.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};

/// GitHub repository the releases are published to.
pub const REPO: &str = "Zqqdd/Archmates";

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Largest release archive we accept to download.
const ARCHIVE_MAX: usize = 64 * 1024 * 1024;

fn http_client(timeout: Duration) -> Result<reqwest::Client> {
    let tls = iroh::tls::CaTlsConfig::embedded()
        .client_config(iroh::tls::default_provider())
        .context("TLS setup failed")?;
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(timeout)
        .user_agent(concat!("archmates/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Fetches the latest released version (without the leading "v").
pub async fn latest_version() -> Result<String> {
    let client = http_client(Duration::from_secs(20))?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("could not reach GitHub")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(anyhow!("no release published yet"));
    }
    let body = response.error_for_status()?.text().await?;
    let json: serde_json::Value = serde_json::from_str(&body)?;
    let tag = json["tag_name"].as_str().ok_or_else(|| anyhow!("unexpected GitHub answer"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

fn parse(v: &str) -> Vec<u64> {
    v.split(|c: char| c == '.' || c == '-')
        .map_while(|p| p.parse().ok())
        .collect()
}

/// True if `candidate` is a strictly newer version than `current` (e.g. 0.10.0 > 0.9.2).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

/// True when this copy is managed by pacman (installed under /usr): pacman must update it.
pub fn managed_by_pacman() -> bool {
    std::env::current_exe().map(|e| e.starts_with("/usr")).unwrap_or(false)
}

/// How this copy was installed, to tell the user how to update it.
pub fn how_to_update() -> String {
    if cfg!(windows) {
        "restart Winmates (it updates itself when it starts)".into()
    } else if managed_by_pacman() {
        "sudo pacman -Syu".into()
    } else {
        "archmates update --install".into()
    }
}

async fn download(client: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>> {
    let response = client.get(url).send().await.with_context(|| format!("downloading {url}"))?;
    let response = response.error_for_status().with_context(|| format!("downloading {url}"))?;
    if response.content_length().is_some_and(|len| len as usize > max) {
        return Err(anyhow!("{url} is too large"));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > max {
        return Err(anyhow!("{url} is too large"));
    }
    Ok(bytes.to_vec())
}

/// Downloads `file` from release `version` and checks it against `file.sha256`.
async fn download_checked(version: &str, file: &str) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};

    if std::env::consts::ARCH != "x86_64" {
        return Err(anyhow!("releases are only built for x86_64; build from source instead"));
    }
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    let client = http_client(Duration::from_secs(300))?;
    println!("→ Downloading {file}");
    let archive = download(&client, &format!("{base}/{file}"), ARCHIVE_MAX).await?;
    let checksum_file = download(&client, &format!("{base}/{file}.sha256"), 4096).await?;
    let expected = String::from_utf8_lossy(&checksum_file)
        .split_whitespace()
        .next()
        .map(str::to_lowercase)
        .ok_or_else(|| anyhow!("empty checksum file"))?;
    let actual: String = Sha256::digest(&archive).iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        return Err(anyhow!("checksum mismatch: the download is corrupted or was tampered with (nothing installed)"));
    }
    println!("→ Checksum OK");
    Ok(archive)
}

/// Windows: downloads release `version`, checks it, and puts the new programs in place
/// (in %LOCALAPPDATA%\Programs\Winmates). The running ones keep working until restarted.
#[cfg(windows)]
pub async fn install(version: &str) -> Result<()> {
    let name = format!("winmates-{version}-x86_64-windows");
    let archive = download_checked(version, &format!("{name}.zip")).await?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).context("unreadable archive")?;
    let mut files = Vec::new();
    for wanted in WINDOWS_PROGRAMS {
        // `<name>/winmates.exe`, whichever separator the archiver used. Only the bytes are
        // taken: where they go is decided here, never by the archive.
        let index = (0..zip.len())
            .find(|&i| {
                zip.name_for_index(i)
                    .is_some_and(|n| n.replace('\\', "/") == format!("{name}/{wanted}"))
            })
            .ok_or_else(|| anyhow!("{wanted} is missing from the archive (nothing installed)"))?;
        let mut entry = zip.by_index(index)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)?;
        files.push((wanted, bytes));
    }
    let dir = crate::platform::install_dir();
    std::fs::create_dir_all(&dir)?;
    for (file, bytes) in files {
        replace_program(&dir.join(file), &bytes)?;
    }
    println!("→ Installed in {}", dir.display());
    Ok(())
}

/// The programs shipped in the Windows archive: the app, WinmatesUpdater, and the service +
/// command line (same as on Linux).
#[cfg(windows)]
pub const WINDOWS_PROGRAMS: [&str; 3] = ["winmates.exe", "winmates-updater.exe", "archmates.exe"];

/// Windows can't overwrite a running program, but it can rename it: the old one is moved
/// aside (deleted at the next start, see `cleanup_old_programs`) and the new one takes its name.
#[cfg(windows)]
pub fn replace_program(target: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if target.exists() {
        let aside = target.with_extension(format!("old-{}", crate::daemon::now_ms()));
        std::fs::rename(target, &aside).with_context(|| format!("cannot replace {}", target.display()))?;
    }
    let tmp = target.with_extension("new");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// Removes programs left aside by an update (those still running stay until next time).
#[cfg(windows)]
pub fn cleanup_old_programs() {
    let Ok(entries) = std::fs::read_dir(crate::platform::install_dir()) else { return };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().contains(".old-") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Linux: downloads release `version`, checks it, and runs its installer. Returns once installed.
#[cfg(not(windows))]
pub async fn install(version: &str) -> Result<()> {
    let name = format!("archmates-{version}-x86_64-linux");
    let archive = download_checked(version, &format!("{name}.tar.gz")).await?;

    // Unpack in a fresh private folder, then run the installer shipped in the archive.
    let work = crate::config::Paths::detect().cache.join("update");
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
    std::fs::read_link("/proc/self/exe")
        .map(|p| p.to_string_lossy().ends_with(" (deleted)"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.2"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }
}
