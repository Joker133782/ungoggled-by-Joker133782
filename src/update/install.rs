use super::{Lock, Paths, Phase, package, releases};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

const CURRENT: &str = "opt/dji-hdmi/current";
const FILES: &[&str] = &[
    "etc/systemd/system/dji-hdmi.service",
    "usr/local/lib/dji-hdmi/prepare-pi.sh",
    "var/lib/dji-hdmi/settings.json",
];
#[derive(Serialize, Deserialize)]
struct Journal {
    previous: Option<PathBuf>,
    files: Vec<bool>,
    target: PathBuf,
}

pub trait Host {
    fn service(&self, verb: &str) -> Result<()>;
    fn migrate(&self, binary: &Path, data: &Path, output: &Path) -> Result<Vec<String>>;
    /// `build` is `None` for the factory release, which reports no build ID.
    /// `Err` carries a human-readable reason (unreachable, wrong
    /// version/build, not up long enough, or timed out) so it can be
    /// persisted into `state.message` for diagnosis.
    fn healthy(&self, version: &str, build: Option<&str>) -> Result<(), String>;
    /// Offline-install any bundled `.deb` packages in `dir` (if it exists
    /// and the dependency is not already present). Returns human-readable
    /// notes for `state.warnings`; never returns an `Err`, because a failure
    /// here must never roll back an otherwise healthy update.
    fn install_packages(&self, dir: &Path) -> Vec<String>;
}
struct System;
impl Host for System {
    fn service(&self, verb: &str) -> Result<()> {
        let mut c = Command::new("timeout");
        c.args(["60", "systemctl", verb]);
        if verb != "daemon-reload" {
            c.arg("dji-hdmi.service");
        }
        ensure!(c.status()?.success(), "systemctl {verb} failed");
        Ok(())
    }
    fn migrate(&self, binary: &Path, data: &Path, output: &Path) -> Result<Vec<String>> {
        let out = Command::new("timeout")
            .arg("20")
            .arg(binary)
            .arg("migrate-settings")
            .arg("--data-dir")
            .arg(data)
            .arg("--output")
            .arg(output)
            .output()?;
        ensure!(
            out.status.success(),
            "Settings migration failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).context("Invalid migration report")
    }
    fn healthy(&self, version: &str, build: Option<&str>) -> Result<(), String> {
        let mut consecutive = 0;
        let mut last_reason = "no attempts made".to_string();
        for _ in 0..40 {
            let outcome = (|| -> Result<serde_json::Value, String> {
                // -L -k: plain HTTP redirects to HTTPS (with a self-signed
                // cert) whenever TLS is enabled, which is the default; curl
                // doesn't fail on a 3xx on its own, so without -L this was
                // "succeeding" with an empty, unparsable body on every
                // single attempt, failing the health check 100% of the time.
                let out = Command::new("curl")
                    .args([
                        "-fsSL",
                        "-k",
                        "--max-time",
                        "2",
                        "http://127.0.0.1/api/status",
                    ])
                    .output()
                    .map_err(|e| format!("curl failed to start: {e}"))?;
                if !out.status.success() {
                    return Err(format!(
                        "curl exit {}: {}",
                        out.status,
                        String::from_utf8_lossy(&out.stderr).trim()
                    ));
                }
                serde_json::from_slice(&out.stdout)
                    .map_err(|e| format!("invalid JSON from /api/status: {e}"))
            })();
            let valid = match &outcome {
                Ok(s) => {
                    let uptime = s["uptime_seconds"].as_u64().unwrap_or(0);
                    let ok = s["version"] == version
                        && s["build_id"] == serde_json::json!(build)
                        && uptime >= 3;
                    if !ok {
                        last_reason = format!(
                            "reachable but not matching yet (wanted version={version} \
                             build_id={build:?}, got version={:?} build_id={:?} \
                             uptime_seconds={uptime})",
                            s["version"], s["build_id"]
                        );
                    }
                    ok
                }
                Err(e) => {
                    last_reason = e.clone();
                    false
                }
            };
            if valid {
                consecutive += 1;
                if consecutive >= 3 {
                    return Ok(());
                }
            } else {
                consecutive = 0;
            }
            thread::sleep(Duration::from_millis(500));
        }
        Err(format!("timed out after 20s; last attempt: {last_reason}"))
    }
    fn install_packages(&self, dir: &Path) -> Vec<String> {
        if !dir.is_dir() {
            return Vec::new();
        }
        // Already present (a fresh image, or an earlier update already did
        // this): skip without even looking at the bundled files.
        if Command::new("mpv")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Vec::new();
        }
        let debs: Vec<_> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "deb"))
            .collect();
        if debs.is_empty() {
            return Vec::new();
        }
        let mut c = Command::new("timeout");
        c.args(["120", "apt-get", "install", "-y", "--no-install-recommends"]);
        c.env("DEBIAN_FRONTEND", "noninteractive");
        c.args(&debs);
        match c.output() {
            Ok(o) if o.status.success() => {
                vec!["Installed mpv for USB-drive video playback".into()]
            }
            Ok(o) => vec![format!(
                "Could not install mpv for USB-drive video playback (core update unaffected): {}",
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .next_back()
                    .unwrap_or("unknown error")
            )],
            Err(e) => vec![format!(
                "Could not run apt-get for mpv ({e}); core update unaffected"
            )],
        }
    }
}
fn lock_wait(paths: &Paths) -> Result<Lock> {
    for _ in 0..20 {
        if let Ok(l) = Lock::acquire(paths) {
            return Ok(l);
        }
        thread::sleep(Duration::from_millis(250));
    }
    Lock::acquire(paths)
}
pub fn install(id: &str, skip_health_check: bool) -> Result<()> {
    let paths = Paths::default();
    let _lock = lock_wait(&paths)?;
    transact(
        &paths,
        id,
        package::architecture(),
        &System,
        skip_health_check,
    )
}
/// Run one install or rollback job and record its outcome in the history.
fn transact(
    paths: &Paths,
    id: &str,
    arch: &str,
    host: &impl Host,
    skip_health_check: bool,
) -> Result<()> {
    let from = releases::current_name(paths);
    let rollback = paths
        .job(id)
        .is_ok_and(|j| j.join("rollback.json").exists());
    let result = apply(paths, id, arch, host, skip_health_check);
    if let Err(e) = &result {
        let mut state = paths.state()?;
        if state.phase != Phase::RolledBack {
            state.phase = Phase::Failed;
            state.message = format!("{e:#}");
            paths.save(&state)?;
        }
    }
    let state = paths.state().unwrap_or_default();
    let outcome = releases::Entry {
        action: if rollback { "rollback" } else { "install" }.into(),
        from,
        to: state.build_id.clone(),
        version: state.version.clone(),
        result: match state.phase {
            Phase::Succeeded => "succeeded",
            Phase::RolledBack => "rolled_back",
            _ => "failed",
        }
        .into(),
        message: state.message.clone(),
        ..Default::default()
    };
    if let Err(e) = releases::record(paths, outcome) {
        eprintln!("Cannot write update history: {e:#}");
    }
    result
}
fn atomic_copy(source: &Path, destination: &Path, mode: u32) -> Result<()> {
    fs::create_dir_all(destination.parent().unwrap())?;
    // Files are bounded by package validation; copy through a synced file.
    let temp = destination.with_extension("updating");
    let mut input = fs::File::open(source)?;
    let mut output = fs::File::create(&temp)?;
    std::io::copy(&mut input, &mut output)?;
    output.set_permissions(fs::Permissions::from_mode(mode))?;
    output.sync_all()?;
    fs::rename(temp, destination)?;
    sync_parent(destination)
}
fn sync_parent(path: &Path) -> Result<()> {
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}
fn set_current(paths: &Paths, target: &Path) -> Result<()> {
    let current = paths.at(CURRENT);
    fs::create_dir_all(current.parent().unwrap())?;
    let next = current.with_extension("next");
    let _ = fs::remove_file(&next);
    symlink(target, &next)?;
    fs::rename(next, &current)?;
    sync_parent(&current)
}
fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn setup_recovery(paths: &Paths) -> Result<()> {
    let helper = paths.at("usr/local/lib/dji-hdmi/update-helper");
    atomic_copy(&std::env::current_exe()?, &helper, 0o755)?;
    let unit = paths.at("etc/systemd/system/ungoggled-update-recovery.service");
    fs::create_dir_all(unit.parent().unwrap())?;
    crate::settings::atomic_write(
        &unit,
        include_bytes!("../../deploy/ungoggled-update-recovery.service"),
    )?;
    // Enable directly: no dependency restart while the transaction is in progress.
    let enabled =
        paths.at("etc/systemd/system/multi-user.target.wants/ungoggled-update-recovery.service");
    fs::create_dir_all(enabled.parent().unwrap())?;
    if enabled.symlink_metadata().is_err() {
        symlink("../ungoggled-update-recovery.service", &enabled)?;
        sync_parent(&enabled)?;
    }
    Ok(())
}
/// What a transaction switches to: a freshly unpacked or an installed release.
struct Candidate {
    dir: PathBuf,
    version: String,
    build: Option<String>,
    /// Service unit and preparation script to install; `None` keeps the
    /// current ones (the factory release does not store copies).
    service: Option<PathBuf>,
    prepare: Option<PathBuf>,
    done: String,
}
fn apply(
    paths: &Paths,
    id: &str,
    arch: &str,
    host: &impl Host,
    skip_health_check: bool,
) -> Result<()> {
    let mut state = paths.state()?;
    ensure!(
        state.id.as_deref() == Some(id)
            && matches!(
                state.phase,
                // RolledBack/Failed: this exact job's own files are still
                // staged (never cleaned up except on success), so a retry
                // — typically with `skip_health_check` after a false-
                // positive health check — can run the same job again
                // rather than needing a fresh upload.
                Phase::Ready | Phase::Queued | Phase::RolledBack | Phase::Failed
            ),
        "No matching prepared update"
    );
    let job = paths.job(id)?;
    if let Ok(bytes) = fs::read(job.join("rollback.json")) {
        let request: releases::Rollback = serde_json::from_slice(&bytes)?;
        let release = releases::rollback_target(paths, &request.release)?;
        let previous = fs::read_link(paths.at(CURRENT)).ok();
        state.phase = Phase::Installing;
        state.message = format!("Switching back to {}", release.version);
        paths.save(&state)?;
        return switch(
            paths,
            &mut state,
            &job,
            Candidate {
                done: format!("Switched back to {} ({})", release.version, release.name),
                dir: release.dir,
                version: release.version,
                build: release.build,
                service: release.service,
                prepare: release.prepare,
            },
            previous,
            host,
            skip_health_check,
        );
    }
    let source = job.join("package");
    let manifest = package::validate(&source, arch)?;
    let build = manifest.release_id()?;
    // The history records what was installed, whatever the upload state held.
    state.version = Some(manifest.version.clone());
    state.build_id = Some(build.clone());
    let target = paths.at(&format!("opt/dji-hdmi/releases/{build}"));
    let previous = fs::read_link(paths.at(CURRENT)).ok();
    ensure!(
        previous.as_ref() != Some(&target)
            && fs::canonicalize(paths.at(CURRENT)).ok().as_ref() != Some(&target),
        "This exact build is already installed"
    );
    state.phase = Phase::Installing;
    state.message = "Preparing application and backing up settings".into();
    paths.save(&state)?;
    // A leftover incomplete release can only be removed if it is not the active release.
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::create_dir_all(target.join("bin"))?;
    for name in manifest.files.keys().filter(|p| p.starts_with("web/")) {
        atomic_copy(&source.join(name), &target.join(name), 0o644)?;
    }
    atomic_copy(
        &source.join(format!("bin/{arch}/ungoggled")),
        &target.join("bin/ungoggled"),
        0o755,
    )?;
    atomic_copy(
        &source.join("manifest.json"),
        &target.join("manifest.json"),
        0o644,
    )?;
    // Keep this release's service files, so a later rollback can restore them.
    atomic_copy(
        &source.join("dji-hdmi.service"),
        &target.join("dji-hdmi.service"),
        0o644,
    )?;
    atomic_copy(
        &source.join("prepare-pi.sh"),
        &target.join("prepare-pi.sh"),
        0o755,
    )?;
    switch(
        paths,
        &mut state,
        &job,
        Candidate {
            service: Some(target.join("dji-hdmi.service")),
            prepare: Some(target.join("prepare-pi.sh")),
            dir: target,
            done: format!("Updated to {}", manifest.version),
            version: manifest.version,
            build: Some(build),
        },
        previous,
        host,
        skip_health_check,
    )?;
    let _ = fs::remove_dir_all(source);
    Ok(())
}
/// Stop, snapshot, migrate settings, activate `next`, restart and verify.
/// Restores the previous release, settings and service files on failure.
fn switch(
    paths: &Paths,
    state: &mut super::State,
    job: &Path,
    next: Candidate,
    previous: Option<PathBuf>,
    host: &impl Host,
    skip_health_check: bool,
) -> Result<()> {
    let target = next.dir.clone();
    setup_recovery(paths)?;
    let backup = job.join("backup");
    fs::create_dir_all(&backup)?;
    // Stop before taking the settings snapshot, so no request can race the backup.
    if let Err(e) = host.service("stop") {
        let _ = host.service("start");
        return Err(e);
    }
    let mut files = Vec::new();
    let snapshot = (|| -> Result<()> {
        for (i, name) in FILES.iter().enumerate() {
            let path = paths.at(name);
            let exists = path.exists();
            files.push(exists);
            if exists {
                atomic_copy(
                    &path,
                    &backup.join(i.to_string()),
                    if i == 1 { 0o755 } else { 0o600 },
                )?;
            }
        }
        let journal = Journal {
            previous: previous.clone(),
            files,
            target: target.clone(),
        };
        crate::settings::atomic_write(&job.join("journal.json"), &serde_json::to_vec(&journal)?)?;
        Ok(())
    })();
    if let Err(e) = snapshot {
        let _ = host.service("start");
        return Err(e);
    }
    let attempt = (|| -> Result<()> {
        let migrated = job.join("settings.json");
        state.warnings = host.migrate(
            &target.join("bin/ungoggled"),
            &paths.at("var/lib/dji-hdmi"),
            &migrated,
        )?;
        atomic_copy(&migrated, &paths.at(FILES[2]), 0o600)?;
        if let Some(service) = &next.service {
            atomic_copy(service, &paths.at(FILES[0]), 0o644)?;
        }
        if let Some(prepare) = &next.prepare {
            atomic_copy(prepare, &paths.at(FILES[1]), 0o755)?;
        }
        if !paths.at("etc/default/dji-hdmi").exists() {
            fs::create_dir_all(paths.at("etc/default"))?;
            crate::settings::atomic_write(
                &paths.at("etc/default/dji-hdmi"),
                b"DJI_HDMI_DECODER=v4l2h264dec\n",
            )?;
        }
        set_current(paths, &target)?;
        state.phase = Phase::Restarting;
        state.message = "Restarting and checking the new release".into();
        paths.save(state)?;
        host.service("daemon-reload")?;
        host.service("enable")?;
        host.service("start")?;
        if skip_health_check {
            // An explicit escape hatch for a health check that is itself
            // broken (it has happened — see CHANGELOG 0.3.34/0.3.35) and
            // would otherwise roll back a perfectly good release forever.
            // Not run at all, so a release that is actually broken keeps
            // running instead of being caught here — flagged clearly since
            // this is the one case where "succeeded" doesn't mean verified.
            state
                .warnings
                .push("Health check skipped at your request — this release was not automatically verified before being kept.".into());
        } else {
            host.healthy(&next.version, next.build.as_deref())
                .map_err(|reason| {
                    anyhow::anyhow!("New release did not pass its health check: {reason}")
                })?;
        }
        // Only a healthy application becomes the recovery helper for the next update.
        atomic_copy(
            &target.join("bin/ungoggled"),
            &paths.at("usr/local/lib/dji-hdmi/update-helper"),
            0o755,
        )?;
        state.phase = Phase::Succeeded;
        state.message = next.done.clone();
        // Best-effort and after the point of no return: whatever happens
        // here, the update itself already succeeded.
        state
            .warnings
            .extend(host.install_packages(&job.join("package/packages")));
        paths.save(state)?;
        Ok(())
    })();
    if let Err(error) = attempt {
        if let Err(rollback) = restore(paths, job, host, false) {
            bail!("Update failed: {error:#}. Automatic rollback failed: {rollback:#}");
        }
        state.phase = Phase::RolledBack;
        state.message = format!("Change failed; previous version and settings restored: {error:#}");
        paths.save(state)?;
        return Err(error);
    }
    // The success record is the commit point. Cleanup cannot roll it back.
    let _ = remove_file(&job.join("journal.json"));
    // Bounded storage matters on small cards; see `releases::prune`.
    releases::prune(paths, &target, previous.as_deref());
    Ok(())
}
fn restore(paths: &Paths, job: &Path, host: &impl Host, boot: bool) -> Result<()> {
    let journal: Journal = serde_json::from_slice(&fs::read(job.join("journal.json"))?)?;
    ensure!(
        journal.files.len() == FILES.len(),
        "Unknown recovery journal format"
    );
    if !boot {
        host.service("stop")?;
    }
    for (i, name) in FILES.iter().enumerate() {
        if journal.files[i] {
            atomic_copy(
                &job.join("backup").join(i.to_string()),
                &paths.at(name),
                if i == 1 {
                    0o755
                } else if i == 2 {
                    0o600
                } else {
                    0o644
                },
            )?;
        } else {
            remove_file(&paths.at(name))?;
        }
    }
    match journal.previous {
        Some(previous) => set_current(paths, &previous)?,
        None => remove_file(&paths.at(CURRENT))?,
    }
    host.service("daemon-reload")?;
    if !boot {
        host.service("start")?;
    }
    remove_file(&job.join("journal.json"))?;
    Ok(())
}
pub fn recover() -> Result<()> {
    let paths = Paths::default();
    let _lock = Lock::acquire(&paths)?;
    recover_with(&paths, &System)
}
fn recover_with(paths: &Paths, host: &impl Host) -> Result<()> {
    let mut state = paths.state()?;
    let Some(id) = &state.id else { return Ok(()) };
    let job = paths.job(id)?;
    if state.phase == Phase::Succeeded {
        remove_file(&job.join("journal.json"))?;
        return Ok(());
    }
    if job.join("journal.json").exists() {
        restore(paths, &job, host, true)?;
        state.phase = Phase::RolledBack;
        state.message =
            "Interrupted update recovered; previous version and settings restored".into();
        paths.save(&state)?;
        let _ = releases::record(
            paths,
            releases::Entry {
                action: "recovery".into(),
                to: releases::current_name(paths),
                version: state.version.clone(),
                result: "rolled_back".into(),
                message: state.message.clone(),
                ..Default::default()
            },
        );
    } else if state.phase.busy() {
        state.phase = Phase::Failed;
        state.message =
            "Update interrupted before installation; previous application retained".into();
        paths.save(&state)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{
        State,
        tests::{Fixture, bundle},
    };
    use std::cell::RefCell;
    struct FakeHost {
        healthy: bool,
        calls: RefCell<Vec<String>>,
        checked: RefCell<Vec<(String, Option<String>)>>,
        packages_checked: RefCell<Vec<PathBuf>>,
    }
    impl FakeHost {
        fn new(healthy: bool) -> Self {
            Self {
                healthy,
                calls: Default::default(),
                checked: Default::default(),
                packages_checked: Default::default(),
            }
        }
    }
    impl Host for FakeHost {
        fn service(&self, verb: &str) -> Result<()> {
            self.calls.borrow_mut().push(verb.into());
            Ok(())
        }
        fn migrate(&self, _: &Path, data: &Path, output: &Path) -> Result<Vec<String>> {
            crate::settings::migrate(data, output)
        }
        fn healthy(&self, version: &str, build: Option<&str>) -> Result<(), String> {
            self.checked
                .borrow_mut()
                .push((version.into(), build.map(Into::into)));
            if self.healthy {
                Ok(())
            } else {
                Err("fake host reported unhealthy".into())
            }
        }
        fn install_packages(&self, dir: &Path) -> Vec<String> {
            self.packages_checked.borrow_mut().push(dir.to_path_buf());
            Vec::new()
        }
    }
    fn setup() -> (Fixture, Paths, PathBuf) {
        let f = Fixture::new();
        let paths = f.paths();
        let old = paths.at("opt/dji-hdmi/releases/old");
        fs::create_dir_all(&old).unwrap();
        set_current(&paths, &old).unwrap();
        for name in FILES {
            let path = paths.at(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, if name.ends_with(".json") { r#"{"hdmi_mode":"1920x1080@30","preview_enabled":false,"fallback_image":null,"retired_option":true}"# } else { "old file" }).unwrap();
        }
        for name in [
            "etc/default/dji-hdmi",
            "etc/shadow",
            "etc/NetworkManager/system-connections/dji-hdmi.nmconnection",
            "var/lib/dji-hdmi/images/custom.png",
        ] {
            let p = paths.at(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "preserve exactly").unwrap();
        }
        bundle(&paths.job("123").unwrap().join("package"));
        paths
            .save(&State {
                id: Some("123".into()),
                phase: Phase::Ready,
                ..State::default()
            })
            .unwrap();
        (f, paths, old)
    }
    #[test]
    fn system_install_packages_skips_a_missing_directory() {
        assert!(
            System
                .install_packages(Path::new("/does/not/exist"))
                .is_empty()
        );
    }
    #[test]
    fn commits_healthy_release_and_preserves_configuration() {
        let (_f, paths, old) = setup();
        let host = FakeHost::new(true);
        apply(&paths, "123", "aarch64", &host, false).unwrap();
        assert_eq!(
            *host.packages_checked.borrow(),
            vec![paths.job("123").unwrap().join("package/packages")]
        );
        assert_ne!(fs::read_link(paths.at(CURRENT)).unwrap(), old);
        assert!(old.exists());
        let state = paths.state().unwrap();
        assert_eq!(state.phase, Phase::Succeeded);
        assert_eq!(state.warnings.len(), 1);
        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(paths.at(FILES[2])).unwrap()).unwrap();
        assert_eq!(settings["hdmi_mode"], "1920x1080@30");
        assert_eq!(settings["preview_enabled"], false);
        for name in [
            "etc/default/dji-hdmi",
            "etc/shadow",
            "etc/NetworkManager/system-connections/dji-hdmi.nmconnection",
            "var/lib/dji-hdmi/images/custom.png",
        ] {
            assert_eq!(
                fs::read_to_string(paths.at(name)).unwrap(),
                "preserve exactly"
            );
        }
        assert!(!paths.job("123").unwrap().join("journal.json").exists());
    }
    #[test]
    fn failed_health_check_restores_previous_release_and_exact_settings() {
        let (_f, paths, old) = setup();
        let before = fs::read(paths.at(FILES[2])).unwrap();
        let host = FakeHost::new(false);
        assert!(apply(&paths, "123", "aarch64", &host, false).is_err());
        assert_eq!(paths.state().unwrap().phase, Phase::RolledBack);
        assert_eq!(fs::read_link(paths.at(CURRENT)).unwrap(), old);
        assert_eq!(fs::read(paths.at(FILES[2])).unwrap(), before);
        assert_eq!(fs::read_to_string(paths.at(FILES[0])).unwrap(), "old file");
        assert_eq!(fs::read_to_string(paths.at(FILES[1])).unwrap(), "old file");
        assert_eq!(host.calls.borrow().last().unwrap(), "start");
    }
    #[test]
    fn skip_health_check_keeps_a_release_the_fake_host_reports_unhealthy_and_retries_a_rolled_back_job()
     {
        let (_f, paths, old) = setup();
        let host = FakeHost::new(false);
        assert!(apply(&paths, "123", "aarch64", &host, false).is_err());
        assert_eq!(paths.state().unwrap().phase, Phase::RolledBack);
        assert_eq!(fs::read_link(paths.at(CURRENT)).unwrap(), old);
        // Retrying the exact same job, now skipping the check, does not need
        // a fresh upload (apply's phase guard accepts RolledBack for this).
        apply(&paths, "123", "aarch64", &host, true).unwrap();
        let state = paths.state().unwrap();
        assert_eq!(state.phase, Phase::Succeeded);
        assert_ne!(fs::read_link(paths.at(CURRENT)).unwrap(), old);
        assert!(
            state
                .warnings
                .iter()
                .any(|w| w.contains("Health check skipped")),
            "{:?}",
            state.warnings
        );
    }
    #[test]
    fn corrupt_settings_abort_and_restore() {
        let (_f, paths, old) = setup();
        fs::write(paths.at(FILES[2]), "invalid JSON").unwrap();
        let host = FakeHost::new(true);
        assert!(apply(&paths, "123", "aarch64", &host, false).is_err());
        assert_eq!(paths.state().unwrap().phase, Phase::RolledBack);
        assert_eq!(fs::read_link(paths.at(CURRENT)).unwrap(), old);
        assert_eq!(
            fs::read_to_string(paths.at(FILES[2])).unwrap(),
            "invalid JSON"
        );
    }
    #[test]
    fn boot_recovers_uncommitted_transaction_without_starting_dependent_service() {
        let (_f, paths, old) = setup();
        let job = paths.job("123").unwrap();
        fs::create_dir(job.join("backup")).unwrap();
        for (i, name) in FILES.iter().enumerate() {
            fs::copy(paths.at(name), job.join("backup").join(i.to_string())).unwrap();
            fs::write(paths.at(name), "interrupted replacement").unwrap();
        }
        let target = paths.at("opt/dji-hdmi/releases/interrupted");
        set_current(&paths, &target).unwrap();
        fs::write(
            job.join("journal.json"),
            serde_json::to_vec(&Journal {
                previous: Some(old.clone()),
                files: vec![true; FILES.len()],
                target,
            })
            .unwrap(),
        )
        .unwrap();
        let mut state = paths.state().unwrap();
        state.phase = Phase::Restarting;
        paths.save(&state).unwrap();
        let host = FakeHost::new(true);
        recover_with(&paths, &host).unwrap();
        assert_eq!(fs::read_link(paths.at(CURRENT)).unwrap(), old);
        assert_eq!(paths.state().unwrap().phase, Phase::RolledBack);
        assert_eq!(*host.calls.borrow(), vec!["daemon-reload"]);
        let history = releases::history(&paths);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].action, "recovery");
        assert!(!job.join("journal.json").exists());
    }
    #[test]
    fn committed_transaction_never_rolls_back_during_boot_cleanup() {
        let (_f, paths, old) = setup();
        let job = paths.job("123").unwrap();
        fs::write(job.join("journal.json"), "cleanup pending").unwrap();
        let mut state = paths.state().unwrap();
        state.phase = Phase::Succeeded;
        paths.save(&state).unwrap();
        let host = FakeHost::new(true);
        recover_with(&paths, &host).unwrap();
        assert_eq!(paths.state().unwrap().phase, Phase::Succeeded);
        assert_eq!(fs::read_link(paths.at(CURRENT)).unwrap(), old);
        assert!(host.calls.borrow().is_empty());
        assert!(!job.join("journal.json").exists());
    }

    const FACTORY: &str = "0.3.1-0123456789ab";
    /// The SD-card image release: binary and VERSION, no manifest or service copies.
    fn with_factory_release() -> (Fixture, Paths, PathBuf) {
        let (f, paths, _) = setup();
        let factory = paths.at(releases::DIRECTORY).join(FACTORY);
        fs::create_dir_all(factory.join("bin")).unwrap();
        fs::write(factory.join("bin/ungoggled"), "factory binary").unwrap();
        fs::write(factory.join("VERSION"), format!("{FACTORY}\n")).unwrap();
        set_current(&paths, &factory).unwrap();
        (f, paths, factory)
    }
    fn queue_rollback(paths: &Paths, id: &str, release: &str) {
        let job = paths.job(id).unwrap();
        fs::create_dir_all(&job).unwrap();
        fs::write(
            job.join("rollback.json"),
            serde_json::to_vec(&releases::Rollback {
                release: release.into(),
            })
            .unwrap(),
        )
        .unwrap();
        paths
            .save(&State {
                id: Some(id.into()),
                phase: Phase::Queued,
                build_id: Some(release.into()),
                ..State::default()
            })
            .unwrap();
    }
    fn current(paths: &Paths) -> String {
        releases::current_name(paths).unwrap()
    }
    #[test]
    fn switch_back_and_forth_between_releases_with_history() {
        let (_f, paths, factory) = with_factory_release();
        let host = FakeHost::new(true);
        transact(&paths, "123", "aarch64", &host, false).unwrap();
        let updated = current(&paths);
        assert_ne!(updated, FACTORY);
        let release = paths.at(releases::DIRECTORY).join(&updated);
        // Each web-installed release keeps its own service files.
        assert_eq!(
            fs::read_to_string(release.join("dji-hdmi.service")).unwrap(),
            "new dji-hdmi.service"
        );
        let listed = releases::list(&paths);
        assert_eq!(listed.len(), 2);
        assert!(
            listed
                .iter()
                .any(|r| r.factory && r.name == FACTORY && !r.current)
        );
        assert!(listed.iter().any(|r| r.current && r.name == updated));

        // Back to the factory release: its binary migrates the settings and the
        // health check expects no build ID.
        queue_rollback(&paths, "124", FACTORY);
        transact(&paths, "124", "aarch64", &host, false).unwrap();
        assert_eq!(current(&paths), FACTORY);
        assert_eq!(
            fs::canonicalize(paths.at(CURRENT)).unwrap(),
            fs::canonicalize(&factory).unwrap()
        );
        // The health check expects the factory release's own (older) version.
        assert_eq!(
            host.checked.borrow().last().unwrap(),
            &("0.3.1".to_owned(), None)
        );
        assert_eq!(paths.state().unwrap().phase, Phase::Succeeded);
        assert!(release.exists(), "the newer release stays available");

        // Forward again to the newer release, restoring its service files.
        fs::write(paths.at(FILES[0]), "changed meanwhile").unwrap();
        queue_rollback(&paths, "125", &updated);
        transact(&paths, "125", "aarch64", &host, false).unwrap();
        assert_eq!(current(&paths), updated);
        assert_eq!(
            fs::read_to_string(paths.at(FILES[0])).unwrap(),
            "new dji-hdmi.service"
        );
        assert_eq!(
            host.checked.borrow().last().unwrap().1.as_deref(),
            Some(updated.as_str())
        );

        let history = releases::history(&paths);
        let summary: Vec<_> = history
            .iter()
            .map(|e| (e.action.as_str(), e.result.as_str(), e.to.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                ("install", "succeeded", Some(updated.as_str())),
                ("rollback", "succeeded", Some(FACTORY)),
                ("rollback", "succeeded", Some(updated.as_str())),
            ]
        );
        assert_eq!(history[1].from.as_deref(), Some(updated.as_str()));
        assert!(history.iter().all(|e| e.time > 0));
    }
    #[test]
    fn failed_switch_back_keeps_current_release_and_settings() {
        let (_f, paths, _) = with_factory_release();
        transact(&paths, "123", "aarch64", &FakeHost::new(true), false).unwrap();
        let updated = current(&paths);
        let settings = fs::read(paths.at(FILES[2])).unwrap();
        queue_rollback(&paths, "124", FACTORY);
        assert!(transact(&paths, "124", "aarch64", &FakeHost::new(false), false).is_err());
        assert_eq!(current(&paths), updated);
        assert_eq!(fs::read(paths.at(FILES[2])).unwrap(), settings);
        assert_eq!(paths.state().unwrap().phase, Phase::RolledBack);
        let last = releases::history(&paths).pop().unwrap();
        assert_eq!(
            (last.action.as_str(), last.result.as_str()),
            ("rollback", "rolled_back")
        );
    }
    #[test]
    fn switch_back_rejects_active_unknown_and_unsafe_names() {
        let (_f, paths, _) = with_factory_release();
        for name in [FACTORY, "0.3.0-ffffffffffff", "../etc", "old"] {
            assert!(releases::rollback_target(&paths, name).is_err(), "{name}");
            queue_rollback(&paths, "124", name);
            assert!(transact(&paths, "124", "aarch64", &FakeHost::new(true), false).is_err());
            assert_eq!(current(&paths), FACTORY);
        }
        assert!(
            releases::history(&paths)
                .iter()
                .all(|e| e.result == "failed")
        );
    }
    #[test]
    fn pruning_keeps_factory_active_previous_and_one_more() {
        let (_f, paths, _) = with_factory_release();
        let dir = paths.at(releases::DIRECTORY);
        let names: Vec<String> = (0..5).map(|i| format!("0.4.{i}-00000000000{i}")).collect();
        for name in &names {
            fs::create_dir_all(dir.join(name).join("bin")).unwrap();
            fs::write(dir.join(name).join("bin/ungoggled"), "x").unwrap();
            fs::write(dir.join(name).join("VERSION"), name).unwrap();
            // Distinct creation times; newest is names[4].
            thread::sleep(Duration::from_millis(20));
        }
        // Only the factory release lacks manifest.json; give the others one.
        for name in &names {
            fs::write(
                dir.join(name).join("manifest.json"),
                serde_json::to_vec(&crate::update::tests::bundle(&paths.at("scratch"))).unwrap(),
            )
            .unwrap();
        }
        releases::prune(&paths, &dir.join(&names[4]), Some(&dir.join(&names[1])));
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        // Directories outside the VERSION-HASH scheme are never touched.
        let mut want = vec![
            "old".to_owned(),
            FACTORY.to_owned(),
            names[1].clone(),
            names[3].clone(),
            names[4].clone(),
        ];
        want.sort();
        assert_eq!(left, want);
    }
    #[test]
    fn history_is_bounded() {
        let (_f, paths, _) = setup();
        for i in 0..60 {
            releases::record(
                &paths,
                releases::Entry {
                    action: "install".into(),
                    message: i.to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let h = releases::history(&paths);
        assert_eq!(h.len(), 50);
        assert_eq!(h[0].message, "10");
    }
}
