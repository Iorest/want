//! The AppIDs the CLI should use, and where they came from.
//!
//! Resolution order, first hit wins: `--appid`, then `WOLFRAM_APPID`, then
//! `~/.config/want/config.toml`. The file exists so a fresh terminal works
//! without editing a shell profile.
//!
//! Any of the three may hold several keys. The free tier meters each AppID
//! separately, so a second key doubles the monthly budget; the client rotates
//! through them and drops one the API rejects.

use std::io::Write;
use std::path::{Path, PathBuf};

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
    pub appids: Vec<String>,
    pub origin: Origin,
}

/// One key or a list: `appid = "X"` and `appid = ["X", "Y"]` both load, so a
/// single-key file written by an older `want` keeps working.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct File {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    appid: Option<OneOrMany>,
}

impl File {
    fn ids(&self) -> Vec<String> {
        let raw: Vec<&str> = match &self.appid {
            Some(OneOrMany::One(one)) => vec![one.as_str()],
            Some(OneOrMany::Many(many)) => many.iter().map(String::as_str).collect(),
            None => Vec::new(),
        };
        collect(raw)
    }
}

/// `~/.config/want/config.toml`
pub fn path() -> Result<PathBuf> {
    let home = home::home_dir().context("cannot determine the home directory")?;
    Ok(home.join(".config").join("want").join("config.toml"))
}

/// First source that yields at least one AppID.
pub fn resolve(flag: Vec<String>) -> Result<Option<Resolved>> {
    let from_flag = collect(flag.iter().map(String::as_str));
    if !from_flag.is_empty() {
        return Ok(Some(Resolved {
            appids: from_flag,
            origin: Origin::Flag,
        }));
    }

    if let Ok(raw) = std::env::var("WOLFRAM_APPID") {
        let from_env = split(&raw);
        if !from_env.is_empty() {
            return Ok(Some(Resolved {
                appids: from_env,
                origin: Origin::Env,
            }));
        }
    }

    let ids = read()?;
    Ok((!ids.is_empty()).then_some(Resolved {
        appids: ids,
        origin: Origin::File,
    }))
}

/// The AppIDs stored on disk, in order. Empty when there is no file.
pub fn read() -> Result<Vec<String>> {
    let path = path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: File = toml::from_str(&text).with_context(|| {
        format!(
            "failed to parse {} (expected `appid = \"...\"` or `appid = [\"...\", \"...\"]`)",
            path.display()
        )
    })?;

    Ok(file.ids())
}

/// Write the AppIDs to the config file. The file holds a secret, so it is
/// created readable by the owner only.
pub fn write(appids: &[String]) -> Result<PathBuf> {
    let appids = collect(appids.iter().map(String::as_str));
    if appids.is_empty() {
        anyhow::bail!("no AppID to store");
    }

    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let appid = if appids.len() == 1 {
        OneOrMany::One(appids[0].clone())
    } else {
        OneOrMany::Many(appids)
    };

    let body =
        toml::to_string(&File { appid: Some(appid) }).context("failed to serialise the config")?;
    let header = "# Written by `want config --set-appid`.\n\
                  # This file holds a secret: keep it 0600.\n\
                  # Several keys spread load across quotas: appid = [\"first\", \"second\"].\n";

    write_private(&path, &format!("{header}{body}"))?;
    Ok(path)
}

/// Write through a temporary file in the same directory, then rename.
///
/// Two reasons: the file is created 0600 rather than chmod'ed from 0644, and a
/// crash mid-write leaves the previous config intact instead of half a TOML file
/// that fails to parse on the next run.
fn write_private(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("toml.tmp");

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(&tmp)
        .with_context(|| format!("failed to open {}", tmp.display()))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to flush {}", tmp.display()))?;
    drop(file);

    std::fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))
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

/// `24WX… (10 chars)` — enough to tell which key is in use, not enough to use it.
pub fn mask(appid: &str) -> String {
    let visible: String = appid.chars().take(4).collect();
    format!("{visible}… ({} chars)", appid.chars().count())
}

/// Accept `a`, `a,b`, `a b` and mixes of those, in the order given, without
/// duplicates. Keys never contain a separator, so splitting is unambiguous.
pub fn split(raw: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for part in raw.split([',', ' ', '\t', '\n', '\r']) {
        let id = part.trim();
        if !id.is_empty() && !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_owned());
        }
    }
    ids
}

fn collect<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for value in values {
        for id in split(value) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_lists_and_drops_duplicates() {
        assert_eq!(super::split("one"), ["one"]);
        assert_eq!(super::split("one,two"), ["one", "two"]);
        assert_eq!(super::split(" one , two\nthree "), ["one", "two", "three"]);
        // Repeating a key must not double its share of the rotation.
        assert_eq!(super::split("one,two,one"), ["one", "two"]);
        assert!(super::split(" , ").is_empty());
    }

    #[test]
    fn a_single_key_file_and_a_list_file_both_load() {
        let one: super::File = toml::from_str("appid = \"ABC\"").unwrap();
        assert_eq!(one.ids(), ["ABC"]);

        let many: super::File = toml::from_str("appid = [\"ABC\", \"DEF\"]").unwrap();
        assert_eq!(many.ids(), ["ABC", "DEF"]);

        let empty: super::File = toml::from_str("").unwrap();
        assert!(empty.ids().is_empty());
    }
}
