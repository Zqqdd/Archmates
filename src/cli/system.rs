//! The service itself: run it, update it, and the universal desktop widget (Linux).

use anyhow::Result;

use archmates::config::Paths;
use archmates::{platform, update};

/// Universal desktop widget mode (Linux only: Winmates is the desktop app on Windows).
#[cfg(not(windows))]
pub fn widget(paths: &Paths, mode: Option<&str>) -> Result<()> {
    use archmates::platform::linux::widget::{self, Mode};
    if let Some(text) = mode {
        std::fs::create_dir_all(&paths.data)?;
        std::fs::write(paths.widget_mode(), text)?;
    }
    let mode = Mode::load(paths);
    let shown = widget::should_show(mode);
    println!("Universal widget: {} ({})", mode.as_str(), if shown { "shown" } else { "hidden" });
    match widget::program() {
        None => println!("  archmates-widget is not installed."),
        Some(p) if shown => println!("  The service starts {} within 15 s.", p.display()),
        Some(_) if mode == Mode::Auto => println!("  Hidden because Noctalia is running (it has its own widget)."),
        Some(_) => {}
    }
    Ok(())
}

#[cfg(windows)]
pub fn widget(_paths: &Paths, _mode: Option<&str>) -> Result<()> {
    println!("The desktop widget is a Linux feature: on Windows, open Winmates.");
    Ok(())
}

pub async fn update(install: bool) -> Result<()> {
    println!("Installed version: {}", update::CURRENT);
    let latest = update::latest_version().await?;
    let newer = update::is_newer(&latest, update::CURRENT);
    // ARCHMATES_UPDATE_FORCE=1 reinstalls the latest release even when up to date (testing).
    let force = std::env::var_os("ARCHMATES_UPDATE_FORCE").is_some();
    if !install {
        if newer {
            println!("✓ Version {latest} is available. Update with:\n  {}", update::how_to_update());
        } else {
            println!("✓ You are up to date (latest release: {latest}).");
        }
    } else if platform::managed_by_pacman() {
        println!("archmates is managed by pacman here: update it with  sudo pacman -Syu");
    } else if newer || force {
        update::install(&latest).await?;
        if cfg!(windows) {
            println!("\n✓ archmates {latest} installed. Restart Winmates to use it.");
        } else {
            println!("\n✓ archmates {latest} installed. The service restarts by itself.");
        }
    } else {
        println!("✓ You are up to date (latest release: {latest}).");
    }
    Ok(())
}
