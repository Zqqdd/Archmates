//! Your profile: what friends see, your picture, sharing RAM / storage.

use anyhow::Result;

use archmates::config::{self, Paths};
use archmates::{profile, stats};

pub fn show() -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&profile::Profile::local())?);
    Ok(())
}

pub fn stats(paths: &Paths, on: bool) -> Result<()> {
    stats::set_sharing(paths, on)?;
    println!("✓ RAM and storage sharing {}.", if on { "on" } else { "off" });
    Ok(())
}

pub fn avatar(paths: &Paths, image: Option<std::path::PathBuf>, remove: bool) -> Result<()> {
    if remove {
        let _ = std::fs::remove_file(paths.avatar());
        println!("✓ Profile picture removed.");
    } else if let Some(image) = image {
        let bytes = std::fs::read(&image).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", image.display()))?;
        let Some(kind) = profile::image_kind(&bytes) else {
            anyhow::bail!("{} is not a PNG, JPEG, WebP or GIF image", image.display());
        };
        if bytes.len() > 30 * 1024 * 1024 {
            anyhow::bail!("the image is larger than 30 MB");
        }
        config::write_atomic(&paths.avatar(), &bytes)?;
        println!("✓ Profile picture set ({kind}, {} KB).", bytes.len() / 1024);
        println!("  Your friends get it within a minute.");
    }
    let user = profile::Profile::local().user;
    match profile::avatar_path(&user) {
        Some(p) => match profile::read_avatar(&user) {
            Some(sent) => println!(
                "Current picture: {}\n  Sent to your friends as a 256×256 image ({} KB).",
                p.display(),
                sent.len() / 1024
            ),
            None => println!("Found {}, but it is not a readable image.", p.display()),
        },
        None => println!(
            "No profile picture found (system account picture, ~/.face…).\n  Set one with: archmates avatar <image>"
        ),
    }
    Ok(())
}
