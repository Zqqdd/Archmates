//! Update check: asks GitHub for the latest release, never downloads or installs anything.
//!
//! Updates themselves go through pacman (AUR package) or `git pull && ./installer.sh`,
//! so every user sees what gets installed on their machine.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};

/// GitHub repository the releases are published to.
pub const REPO: &str = "Zqqdd/Archmates";

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Fetches the latest released version (without the leading "v").
pub async fn latest_version() -> Result<String> {
    let tls = iroh::tls::CaTlsConfig::embedded()
        .client_config(iroh::tls::default_provider())
        .context("TLS setup failed")?;
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("archmates/", env!("CARGO_PKG_VERSION")))
        .build()?;
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

/// How this copy was installed, to tell the user how to update it.
pub fn how_to_update() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    if exe.starts_with("/usr") {
        "paru -Syu   (or: yay -Syu)".into()
    } else {
        format!(
            "download the latest archive from https://github.com/{REPO}/releases/latest \
             and run ./installer.sh in it (or, from a git clone: git pull && ./installer.sh)"
        )
    }
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
