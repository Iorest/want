//! The AppID the CLI should use, and where it came from.
//!
//! Resolution order, first hit wins: `--appid`, then `WOLFRAM_APPID`, then
//! `~/.config/want/config.toml`. The file exists so a fresh terminal works
//! without editing a shell profile.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Where an AppID was found. Printed by `want config`, so a user can see why a
/// key is or is not being picked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Flag,
    Env,
    File,
}

impl Origin {
    pub fn describe(self) -> &'static str {
        match self {
            Origin::Flag => "--appid",
            Origin::Env => "WOLFRAM_APPID",
            Origin::File => "config file",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub appid: String,
    pub origin: Origin,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct File {
    appid: Option<String>,
}

/// `~/.config/want/config.toml`
pub fn path() -> Result<PathBuf> {
    let home = home::home_dir().context("cannot determine the home directory")?;
    Ok(home.join(".config").join("want").join("config.toml"))
}

/// First source that yields a non-empty AppID.
pub fn resolve(flag: Option<String>) -> Result<Option<Resolved>> {
    if let Some(appid) = flag.filter(|id| !id.trim().is_empty()) {
        return Ok(Some(Resolved {
            appid,
            origin: Origin::Flag,
        }));
    }

    if let Some(appid) = std::env::var("WOLFRAM_APPID")
        .ok()
        .filter(|id| !id.trim().is_empty())
    {
        return Ok(Some(Resolved {
            appid,
            origin: Origin::Env,
        }));
    }

    Ok(read()?.map(|appid| Resolved {
        appid,
        origin: Origin::File,
    }))
}

/// The AppID stored on disk, if the file exists and parses.
pub fn read() -> Result<Option<String>> {
    let path = path()?;
    if !path.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: File = toml::from_str(&text).with_context(|| {
        format!(
            "failed to parse {} (expected `appid = \"...\"`)",
            path.display()
        )
    })?;

    Ok(file.appid.filter(|id| !id.trim().is_empty()))
}

/// Write the AppID to the config file. The file holds a secret, so it is created
/// readable by the owner only.
pub fn write(appid: &str) -> Result<PathBuf> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let body = toml::to_string(&File {
        appid: Some(appid.to_owned()),
    })
    .context("failed to serialise the config")?;
    let header =
        "# Written by `want config --set-appid`.\n# This file holds a secret: keep it 0600.\n";

    std::fs::write(&path, format!("{header}{body}"))
        .with_context(|| format!("failed to write {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", path.display()))?;
    }

    Ok(path)
}

/// Warn when the config file is readable by other users.
pub fn warn_if_world_readable(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            tracing::warn!(
                path = %path.display(),
                "this file holds an AppID but is readable by other users; run `chmod 600`"
            );
        }
    }
}

/// `24WX…(10 chars)` — enough to tell which key is in use, not enough to use it.
pub fn mask(appid: &str) -> String {
    let visible: String = appid.chars().take(4).collect();
    format!("{visible}… ({} chars)", appid.chars().count())
}
