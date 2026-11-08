pub mod api;
mod install;
pub mod package;
pub mod releases;
use anyhow::{Result, bail, ensure};
pub use install::{install, recover};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub const DIRECTORY: &str = "/var/lib/ungoggled-update";
#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub updates: PathBuf,
}
impl Default for Paths {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/"),
            updates: PathBuf::from(DIRECTORY),
        }
    }
}
impl Paths {
    pub fn at(&self, p: &str) -> PathBuf {
        self.root.join(p.trim_start_matches('/'))
    }
    pub fn state(&self) -> Result<State> {
        match fs::read(self.updates.join("state.json")) {
            Ok(b) => Ok(serde_json::from_slice(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, state: &State) -> Result<()> {
        crate::settings::atomic_write(
            &self.updates.join("state.json"),
            &serde_json::to_vec_pretty(state)?,
        )
    }
    pub fn job(&self, id: &str) -> Result<PathBuf> {
        ensure!(
            !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_digit()),
            "Invalid update ID"
        );
        Ok(self.updates.join(id))
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Ready,
    Queued,
    Installing,
    Restarting,
    Succeeded,
    RolledBack,
    Failed,
}
impl Phase {
    pub fn busy(&self) -> bool {
        matches!(self, Self::Queued | Self::Installing | Self::Restarting)
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub id: Option<String>,
    pub phase: Phase,
    pub version: Option<String>,
    pub message: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub release_notes: Option<String>,
    pub build_id: Option<String>,
}
pub struct Lock(fs::File);
impl Lock {
    pub fn acquire(paths: &Paths) -> Result<Self> {
        fs::create_dir_all(&paths.updates)?;
        fs::set_permissions(&paths.updates, fs::Permissions::from_mode(0o700))?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(paths.updates.join("lock"))?;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "Another update operation is in progress"
        );
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
pub fn supported() -> bool {
    (unsafe { libc::geteuid() == 0 })
        && package::architecture() != "unsupported"
        && Path::new("/usr/local/lib/dji-hdmi/update-helper").is_file()
        && Path::new("/run/systemd/system").is_dir()
}
pub fn information(paths: &Paths) -> Value {
    match paths.state() {
        Ok(state) => json!({"available":supported(), "current_version":env!("CARGO_PKG_VERSION"),
            "current_build":running_build(), "architecture":package::architecture(), "max_upload_bytes":package::MAX_UPLOAD,
            "repository":"https://github.com/TimoLehnertz/ungoggled", "state":state}),
        Err(e) => {
            json!({"available":false,"current_version":env!("CARGO_PKG_VERSION"), "error":format!("Cannot read update status: {e}")})
        }
    }
}
pub fn running_build() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let m: package::Manifest =
        serde_json::from_slice(&fs::read(exe.parent()?.parent()?.join("manifest.json")).ok()?)
            .ok()?;
    m.release_id().ok()
}
pub fn available_space(path: &Path) -> Result<()> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    ensure!(
        unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) } == 0,
        "Cannot check available disk space"
    );
    let st = unsafe { st.assume_init() };
    ensure!(
        u128::from(st.f_bavail) * u128::from(st.f_frsize) >= 640 * 1024 * 1024,
        "At least 640 MiB of free disk space is required"
    );
    Ok(())
}
pub fn new_job(paths: &Paths) -> Result<(String, PathBuf)> {
    ensure!(!paths.state()?.phase.busy(), "An update is being installed");
    let previous = paths.state()?.id;
    if let Some(id) = &previous {
        ensure!(
            !paths.job(id)?.join("journal.json").exists(),
            "An interrupted update needs recovery; reboot the Pi before uploading again"
        );
    }
    // A disconnected upload may leave a partial file; reclaim it before space checks.
    for e in fs::read_dir(&paths.updates)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if Some(&name) != previous.as_ref()
            && name.bytes().all(|c| c.is_ascii_digit())
            && e.file_type()?.is_dir()
        {
            fs::remove_dir_all(e.path())?;
        }
    }
    available_space(&paths.updates)?;
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let dir = paths.job(&id)?;
    fs::create_dir(&dir)?;
    Ok((id, dir))
}
pub fn prepare(paths: &Paths, id: &str, archive: &Path, arch: &str) -> Result<State> {
    let job = paths.job(id)?;
    let result = package::unpack(archive, &job.join("package"), arch);
    let _ = fs::remove_file(archive);
    let manifest = match result {
        Ok(m) => m,
        Err(e) => {
            let _ = fs::remove_dir_all(&job);
            return Err(e);
        }
    };
    ready(paths, id, &manifest)
}
fn ready(paths: &Paths, id: &str, m: &package::Manifest) -> Result<State> {
    let notes = fs::read_to_string(paths.job(id)?.join("package/RELEASE_NOTES.md"))?;
    ensure!(notes.len() <= 128 * 1024, "Release notes are too large");
    let state = State {
        id: Some(id.into()),
        phase: Phase::Ready,
        version: Some(m.version.clone()),
        message: "Update checked and ready to install".into(),
        release_notes: Some(notes),
        build_id: Some(m.release_id()?),
        ..State::default()
    };
    // Keep the last job while preparing its replacement. Discard other abandoned uploads.
    let previous = paths.state()?.id;
    paths.save(&state)?;
    for e in fs::read_dir(&paths.updates)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name != id
            && Some(&name) != previous.as_ref()
            && name.bytes().all(|c| c.is_ascii_digit())
            && e.file_type()?.is_dir()
        {
            let _ = fs::remove_dir_all(e.path());
        }
    }
    Ok(state)
}
pub fn launch(paths: &Paths, id: &str, skip_health_check: bool) -> Result<()> {
    ensure!(supported(), "Web updates require an installed Pi service");
    let mut state = paths.state()?;
    ensure!(
        state.id.as_deref() == Some(id)
            // RolledBack/Failed: retrying the same job (typically with
            // `skip_health_check`, after a false-positive health check),
            // not a fresh upload — see install::apply's matching guard.
            && matches!(
                state.phase,
                Phase::Ready | Phase::RolledBack | Phase::Failed
            ),
        "Upload and validate an update first"
    );
    state.phase = Phase::Queued;
    state.message = "Starting update".into();
    paths.save(&state)?;
    let exe = std::env::current_exe()?;
    let mut command = Command::new("systemd-run");
    command.args([
        "--quiet",
        "--collect",
        &format!("--unit=ungoggled-update-{id}"),
        "--property=Type=exec",
        "--property=TimeoutStopSec=90",
    ]);
    command.arg(exe).args(["update-worker", "--id", id]);
    if skip_health_check {
        command.arg("--skip-health-check");
    }
    let output = command.output();
    match output {
        Ok(out) if out.status.success() => Ok(()),
        other => {
            state.phase = Phase::Failed;
            state.message = match other {
                Ok(o) => format!(
                    "Cannot start updater: {}",
                    String::from_utf8_lossy(&o.stderr)
                ),
                Err(e) => e.to_string(),
            };
            paths.save(&state)?;
            bail!("{}", state.message)
        }
    }
}
/// Queue a switch back to an installed release, through the same transaction
/// as an update (snapshot, migration, health check, automatic restore).
pub fn request_rollback(paths: &Paths, release: &str) -> Result<String> {
    ensure!(supported(), "Web updates require an installed Pi service");
    let target = releases::rollback_target(paths, release)?;
    let (id, job) = new_job(paths)?;
    crate::settings::atomic_write(
        &job.join("rollback.json"),
        &serde_json::to_vec(&releases::Rollback {
            release: target.name.clone(),
        })?,
    )?;
    paths.save(&State {
        id: Some(id.clone()),
        phase: Phase::Ready,
        version: Some(target.version.clone()),
        message: format!("Switching back to {}", target.version),
        build_id: Some(target.name.clone()),
        ..State::default()
    })?;
    launch(paths, &id, false)?;
    Ok(id)
}
pub fn releases_information(paths: &Paths) -> Value {
    let mut history = releases::history(paths);
    history.reverse();
    json!({"releases":releases::list(paths),"history":history})
}
pub fn manual(source: &Path) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "Run the installer as root");
    let paths = Paths::default();
    let lock = Lock::acquire(&paths)?;
    let manifest = package::validate(source, package::architecture())?;
    let (id, job) = new_job(&paths)?;
    fs::create_dir(job.join("package"))?;
    for name in manifest
        .files
        .keys()
        .chain(std::iter::once(&"manifest.json".to_owned()))
    {
        let target = job.join("package").join(name);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(source.join(name), target)?;
    }
    ready(&paths, &id, &manifest)?;
    // Copy to a stable path and launch outside the receiver/SSH process group.
    let worker = job.join("worker");
    fs::copy(std::env::current_exe()?, &worker)?;
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755))?;
    let mut state = paths.state()?;
    state.phase = Phase::Queued;
    state.message = "Starting update".into();
    paths.save(&state)?;
    let out = Command::new("systemd-run")
        .args([
            "--quiet",
            "--collect",
            &format!("--unit=ungoggled-update-{id}"),
            "--property=Type=exec",
        ])
        .arg(worker)
        .args(["update-worker", "--id", &id])
        .output();
    if !matches!(&out, Ok(o) if o.status.success()) {
        state.phase = Phase::Failed;
        state.message = match out {
            Ok(o) => String::from_utf8_lossy(&o.stderr).into_owned(),
            Err(e) => format!("Cannot start updater: {e}"),
        };
        paths.save(&state)?;
        bail!("{}", state.message);
    }
    drop(lock);
    println!(
        "Installing {}. Open http://192.168.50.1 to follow the update.",
        manifest.version
    );
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
