//! Updates.
//!
//! - The service only *checks* GitHub once a day and notifies: it never installs anything by itself.
//! - `archmates update --install`, run by the user, downloads the latest release archive from
//!   GitHub over HTTPS, checks its SHA-256 checksum, and runs the installer it contains.
//! - Windows: Winmates does that by itself each time it starts (same download, same
//!   checksum check), then puts the new programs in place of the old ones.
//! The OS-specific installation step is in `platform::{linux,windows}`.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};

/// GitHub repository the releases are published to.
pub const REPO: &str = "Zqqdd/Archmates";

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Largest release archive we accept to download.
const ARCHIVE_MAX: usize = 64 * 1024 * 1024;

fn http_client(timeout: Duration) -> Result<reqwest::Client> {
    let tls =
        iroh::tls::CaTlsConfig::embedded().client_config(iroh::tls::default_provider()).context("TLS setup failed")?;
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
    v.split(|c: char| c == '.' || c == '-').map_while(|p| p.parse().ok()).collect()
}

/// True if `candidate` is a strictly newer version than `current` (e.g. 0.10.0 > 0.9.2).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

/// How to update this copy, for the "update available" notification.
pub fn how_to_update() -> String {
    crate::platform::update_hint()
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
pub(crate) async fn download_checked(version: &str, file: &str) -> Result<Vec<u8>> {
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

/// Downloads release `version`, checks its SHA-256, and installs it (see
/// `platform::install_release`: the Linux installer, or the Windows programs replaced in place).
pub async fn install(version: &str) -> Result<()> {
    crate::platform::install_release(version).await
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
