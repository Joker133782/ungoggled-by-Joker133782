//! Installed application releases and the update history.
//!
//! Releases live in `/opt/dji-hdmi/releases/VERSION-HASH`. Web-installed ones
//! carry their `manifest.json`; the release from the SD-card image carries only
//! `VERSION` and is kept as the factory release. The history is a small JSON
//! log next to the update state, written once per transaction.
use super::{Paths, package};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const DIRECTORY: &str = "opt/dji-hdmi/releases";
const CURRENT: &str = "opt/dji-hdmi/current";
const HISTORY_LIMIT: usize = 50;
/// Web-installed releases kept besides the active, the preceding and the
/// factory release.
const EXTRA_KEPT: usize = 1;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Release {
    pub name: String,
    pub version: String,
    /// Build ID as reported by `/api/status`; `None` for the factory release.
    pub build: Option<String>,
    /// Unix seconds the release directory was created.
    pub installed: Option<u64>,
    pub current: bool,
    pub factory: bool,
    /// Service unit and Pi preparation script shipped with this release.
    #[serde(skip)]
    pub service: Option<PathBuf>,
    #[serde(skip)]
    pub prepare: Option<PathBuf>,
    #[serde(skip)]
    pub dir: PathBuf,
}

/// `VERSION-HASH` with a semantic version and a 12-digit hex build hash.
pub fn managed_name(name: &str) -> bool {
    name.rsplit_once('-').is_some_and(|(version, hash)| {
        semver::Version::parse(version).is_ok()
            && hash.len() == 12
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

pub fn current_name(paths: &Paths) -> Option<String> {
    fs::read_link(paths.at(CURRENT))
        .ok()?
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

pub fn describe(paths: &Paths, name: &str) -> Option<Release> {
    if !managed_name(name) {
        return None;
    }
    let dir = paths.at(DIRECTORY).join(name);
    if !dir.join("bin/ungoggled").is_file() {
        return None;
    }
    let (version, build, factory) = match fs::read(dir.join("manifest.json")) {
        Ok(bytes) => {
            let m: package::Manifest = serde_json::from_slice(&bytes).ok()?;
            let build = m.release_id().ok()?;
            (m.version, Some(build), false)
        }
        Err(_) => {
            let v = fs::read_to_string(dir.join("VERSION")).ok()?;
            let (version, _) = v.trim().rsplit_once('-')?;
            semver::Version::parse(version).ok()?;
            (version.to_owned(), None, true)
        }
    };
    let installed = fs::metadata(&dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let existing = |file: &str| Some(dir.join(file)).filter(|p| p.is_file());
    Some(Release {
        name: name.to_owned(),
        version,
        build,
        installed,
        current: current_name(paths).as_deref() == Some(name),
        factory,
        service: existing("dji-hdmi.service"),
        prepare: existing("prepare-pi.sh"),
        dir,
    })
}

/// Newest first.
pub fn list(paths: &Paths) -> Vec<Release> {
    let mut releases: Vec<Release> = fs::read_dir(paths.at(DIRECTORY))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| describe(paths, &e.file_name().to_string_lossy()))
        .collect();
    releases.sort_by(|a, b| b.installed.cmp(&a.installed).then(b.name.cmp(&a.name)));
    releases
}

/// Remove old web-installed releases. Keeps the active and preceding release,
/// the factory release and the newest `EXTRA_KEPT` others.
pub fn prune(paths: &Paths, active: &Path, previous: Option<&Path>) {
    let canonical = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let keep = [Some(canonical(active)), previous.map(canonical)];
    let mut extra = 0;
    for release in list(paths) {
        let dir = canonical(&release.dir);
        if release.factory || keep.contains(&Some(dir)) {
            continue;
        }
        if extra < EXTRA_KEPT {
            extra += 1;
            continue;
        }
        let _ = fs::remove_dir_all(&release.dir);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Unix seconds.
    pub time: u64,
    /// `install`, `rollback` or `recovery`.
    pub action: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub version: Option<String>,
    /// `succeeded`, `rolled_back` or `failed`.
    pub result: String,
    pub message: String,
}

fn history_path(paths: &Paths) -> PathBuf {
    paths.updates.join("history.json")
}

pub fn history(paths: &Paths) -> Vec<Entry> {
    fs::read(history_path(paths))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn record(paths: &Paths, mut entry: Entry) -> Result<()> {
    entry.time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut log = history(paths);
    log.push(entry);
    let excess = log.len().saturating_sub(HISTORY_LIMIT);
    log.drain(..excess);
    crate::settings::atomic_write(&history_path(paths), &serde_json::to_vec_pretty(&log)?)
}

#[derive(Serialize, Deserialize)]
pub struct Rollback {
    pub release: String,
}

/// Validate a rollback target: installed, intact and not already active.
pub fn rollback_target(paths: &Paths, name: &str) -> Result<Release> {
    let release = describe(paths, name).context("This version is not installed")?;
    ensure!(!release.current, "This version is already active");
    Ok(release)
}
