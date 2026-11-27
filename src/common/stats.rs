//! RAM and storage usage, shared with your friends (turn off with `archmates stats off`).

use serde::{Deserialize, Serialize};

use crate::common::config::Paths;

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

/// Current usage, or None when sharing is turned off.
pub fn current(paths: &Paths) -> Option<Stats> {
    if !sharing_enabled(paths) {
        return None;
    }
    let (ram_used, ram_total) = crate::platform::memory_usage().unwrap_or_default();
    let (disk_used, disk_total) = crate::platform::disk_usage().unwrap_or_default();
    Some(Stats { ram_used, ram_total, disk_used, disk_total })
}
