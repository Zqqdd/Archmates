//! RAM and storage usage, shared with your friends (turn off with `archmates stats off`).

use serde::{Deserialize, Serialize};

use crate::config::Paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub ram_used: u64,
    pub ram_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
}

/// Sharing is on unless ~/.local/share/archmates/no-stats exists.
pub fn sharing_enabled(paths: &Paths) -> bool {
    !paths.data.join("no-stats").exists()
}

pub fn set_sharing(paths: &Paths, on: bool) -> std::io::Result<()> {
    let flag = paths.data.join("no-stats");
    if on {
        let _ = std::fs::remove_file(flag);
        Ok(())
    } else {
        std::fs::create_dir_all(&paths.data)?;
        std::fs::write(flag, b"")
    }
}

fn ram() -> Option<(u64, u64)> {
    let txt = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| -> Option<u64> {
        txt.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
    };
    let total = field("MemTotal:")? * 1024;
    let available = field("MemAvailable:")? * 1024;
    Some((total.saturating_sub(available), total))
}

fn disk() -> Option<(u64, u64)> {
    let out = std::process::Command::new("df").args(["-B1", "--output=size,used", "/"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut nums = text.lines().last()?.split_whitespace().map(|n| n.parse::<u64>().ok());
    let total = nums.next()??;
    let used = nums.next()??;
    Some((used, total))
}

/// Current usage, or None when sharing is turned off.
pub fn current(paths: &Paths) -> Option<Stats> {
    if !sharing_enabled(paths) {
        return None;
    }
    let (ram_used, ram_total) = ram().unwrap_or_default();
    let (disk_used, disk_total) = disk().unwrap_or_default();
    Some(Stats { ram_used, ram_total, disk_used, disk_total })
}
