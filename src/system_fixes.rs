//! System-level fixes outside the application release: boot configuration
//! (`config.txt`) and the patched HEVC decoder module with the reserved memory
//! pool it uses.
//!
//! Field Pis receive these through the normal update without any help from
//! the installed (possibly much older) updater: every updater installs the new
//! `prepare-pi.sh`, which runs `ungoggled system-fixes apply` as root before
//! every service start, and the data lives inside this binary. The image build
//! runs the same command inside the image (`--offline`), so a fresh image and
//! an updated Pi end up in the same state.
//!
//! Nothing here reboots. `config.txt`, the overlay and the module take effect
//! with the next boot; [`pending`] compares what is configured with what is
//! running so the web interface can ask for a reboot.
//!
//! Way back: `ungoggled system-fixes remove-module` removes the module, its
//! overlay line and file and the kernel package hold, and keeps them off
//! (`enable-module` undoes that). `core_freq_min` and the CMA size are plain
//! `config.txt` lines, as are `hdmi_enable_4kp60` and the `gpio-fan` overlay
//! line (a value or overlay someone set outside the managed block wins); the
//! file from before the first change is kept as `config.txt.ungoggled-orig`.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

/// The kernel the embedded module was built for, see
/// `scripts/build-hevc-module.sh` (kernel bump instructions are there).
pub const KERNEL_RELEASE: &str = "6.18.50+rpt-rpi-v8";
/// Debian version of `linux-image-KERNEL_RELEASE` it was built against. A
/// rebuild under the same release name would change symbol CRCs.
pub const KERNEL_PACKAGE: &str = "1:6.18.50-1+rpt1";
const MODULE: &[u8] = include_bytes!("../assets/kernel/6.18.50+rpt-rpi-v8/rpi-hevc-dec.ko");
#[cfg(test)]
const BUILD_INFO: &str = include_str!("../assets/kernel/6.18.50+rpt-rpi-v8/BUILD-INFO");
const OVERLAY: &[u8] = include_bytes!("../assets/kernel/ungoggled-hevc-cma.dtbo");
const OVERLAY_NAME: &str = "ungoggled-hevc-cma";
/// Below `lib/modules/RELEASE/`. depmod searches `updates/` before the
/// stock `kernel/` tree, so this copy wins; the stock module stays untouched.
const MODULE_DIR: &str = "updates/ungoggled";
const MODULE_FILE: &str = "rpi-hevc-dec.ko";
const STOCK_MODULE: &str = "kernel/drivers/media/platform/raspberrypi/hevc_dec/rpi-hevc-dec.ko";
/// Keeps the VideoCore core clock, which also clocks the HVS display scaler,
/// at 500 MHz. Under the ondemand governor the firmware lowers it with the
/// ARM clock (274-324 MHz measured during hardware video decode), and vc4
/// never asks for more than the pixel clock outside a modeset, because its
/// load estimate ignores the source size of a downscaled plane. Downscaling a
/// 4096x3072 action-cam clip onto 1080p then underflows the HVS FIFO (black
/// screen): EUFLOW only ever seen below 325 MHz, never at 500 (FINDINGS.MD).
pub const CORE_FREQ_MIN: &str = "500";
/// HDMI0 4Kp60 (`hdmi_enable_4kp60=1`): the firmware then runs the core clock
/// at 550 MHz, which plays big action-cam clips a few frames per second faster
/// (FINDINGS.MD), and lists the monitor's 4K 50/60 Hz modes. Only the first
/// HDMI port (HDMI0, next to the USB-C port) can do 4Kp60. Never combine it
/// with `hevc_freq` >= 600 or an `over_voltage_delta`: that killed the
/// VideoCore firmware on the test Pi.
pub const HDMI_4KP60: &str = "1";
/// The case or hat fan on a GPIO pin, switched on by the kernel's `gpio-fan`
/// overlay (what raspi-config's "Performance Options > Fan" writes) from this
/// temperature, in millidegrees Celsius. A Pi without a fan on GPIO 14 is not
/// affected: the pin carries the unused UART transmit line. Left out when the
/// app's settings use GPIO 14, when `enable_uart=1` puts the serial console
/// there, or when `config.txt` carries the [`NO_FAN`] line outside the block.
pub const FAN_GPIO: u32 = 14;
pub const FAN_ON_MILLI_C: u32 = 60_000;
const FAN_OVERLAY: &str = "gpio-fan";
/// The owner's opt-out from our fan line: this comment anywhere outside the
/// managed block (the firmware ignores it, and it survives every run).
pub const NO_FAN: &str = "# ungoggled: no-fan";
/// The app's settings, for the GPIO pins they use (default `--data-dir`).
const SETTINGS: &str = "var/lib/dji-hdmi/settings.json";
const BOOT_ID: &str = "proc/sys/kernel/random/boot_id";
/// 640 MiB default CMA pool (`vc4-kms-v3d,cma-size=`). The default 512 MiB is
/// too small for GStreamer `v4l2slh265dec`, which pre-allocates a DPB sized
/// from the stream's declared level before the decoder gets its own ~45 MiB
/// (STREAMON fails with ENOMEM on e.g. DJI Osmo Nano 3840x2880 10-bit). Never
/// more without testing on hardware: on a 2 GiB Pi 4, 768 MiB overflows the
/// space below the 1 GiB boundary the HVS can scan out from and the
/// reservation silently falls back to ~8 MiB. Only ever raised, never lowered.
pub const CMA_SIZE: u64 = 0x2800_0000;
const BOOT: &str = "boot/firmware";
const STATE: &str = "var/lib/dji-hdmi/system-fixes.json";
const DECODER_NODE: &str = "proc/device-tree/scb/codec@7eb10000";
const BEGIN: &str = "# >>> ungoggled system fixes (managed automatically, edits here are replaced)";
const END: &str = "# <<< ungoggled system fixes";
const MAX_CONFIG: usize = 64 * 1024;

pub trait Host {
    /// Run a program to completion; `Ok(stdout)` only for exit status 0.
    fn run(&self, program: &str, args: &[&str]) -> Result<String>;
}
pub struct System;
impl Host for System {
    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        let out = Command::new("timeout")
            .arg("30")
            .arg(program)
            .args(args)
            .output()
            .with_context(|| format!("cannot run {program}"))?;
        if !out.status.success() {
            bail!(
                "{program} {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

pub struct Options {
    pub root: PathBuf,
    /// Kernel release the module has to fit: the running one on a Pi, the
    /// image's own one in an image build.
    pub kernel_release: String,
    /// A running system: may load modules and look at `/sys` and `/proc`.
    pub live: bool,
    /// Run `apt-mark` (only meaningful when `root` is the system's own root).
    pub packages: bool,
}
impl Options {
    pub fn system(root: &Path, kernel_release: Option<String>, offline: bool) -> Result<Self> {
        let kernel_release = match kernel_release {
            Some(r) => r,
            None if offline => bail!("--offline needs --kernel-release"),
            None => fs::read_to_string(root.join("proc/sys/kernel/osrelease"))
                .context("cannot read the running kernel release")?
                .trim()
                .to_owned(),
        };
        Ok(Self {
            root: root.to_path_buf(),
            kernel_release,
            live: !offline,
            packages: root == Path::new("/"),
        })
    }
    fn boot(&self) -> PathBuf {
        self.root.join(BOOT)
    }
    fn modules(&self) -> PathBuf {
        self.root.join("lib/modules").join(KERNEL_RELEASE)
    }
    fn installed_module(&self) -> PathBuf {
        self.modules().join(MODULE_DIR).join(MODULE_FILE)
    }
    fn overlay_file(&self) -> PathBuf {
        self.boot()
            .join("overlays")
            .join(format!("{OVERLAY_NAME}.dtbo"))
    }
}

#[derive(Default, Serialize, Deserialize, PartialEq, Clone, Debug)]
struct State {
    /// Set when the module was switched off (load failure or by hand); only
    /// applies to the exact module build it names.
    #[serde(default)]
    module_disabled: Option<Disabled>,
    /// Packages this code put on hold, so only those get released again.
    #[serde(default)]
    held: Vec<String>,
    /// The last failed install of this module build, so a failure that keeps
    /// happening (a foreign depmod rule, a full root filesystem) is retried
    /// once per boot instead of before every service start.
    #[serde(default)]
    install_failed: Option<Failed>,
}
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
struct Disabled {
    module_sha256: String,
    reason: String,
}
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
struct Failed {
    module_sha256: String,
    boot_id: String,
    reason: String,
}
impl State {
    fn load(o: &Options) -> Self {
        fs::read(o.root.join(STATE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn save(&self, o: &Options) -> Result<()> {
        let path = o.root.join(STATE);
        fs::create_dir_all(path.parent().unwrap())?;
        replace_file(&path, &serde_json::to_vec_pretty(self)?, || Ok(()))
    }
    fn disabled(&self) -> Option<&str> {
        self.module_disabled
            .as_ref()
            .filter(|d| d.module_sha256 == module_sha256())
            .map(|d| d.reason.as_str())
    }
    /// Why this module build failed to install earlier in this boot, if so.
    fn failed_this_boot(&self, o: &Options) -> Option<&str> {
        let boot = boot_id(o)?;
        self.install_failed
            .as_ref()
            .filter(|f| f.module_sha256 == module_sha256() && f.boot_id == boot)
            .map(|f| f.reason.as_str())
    }
}
/// The running boot's id; `None` offline (an image build always tries).
fn boot_id(o: &Options) -> Option<String> {
    if !o.live {
        return None;
    }
    let id = fs::read_to_string(o.root.join(BOOT_ID)).ok()?;
    Some(id.trim().to_owned()).filter(|id| !id.is_empty())
}
/// Save the state if it changed since the last save. A failed save is noted
/// once and not retried unless the state changes again.
fn save_state(o: &Options, state: &State, saved: &mut State, r: &mut Report) {
    if state == saved {
        return;
    }
    if let Err(e) = state.save(o) {
        r.note(format!("Cannot save {STATE}: {e:#}"));
    }
    *saved = state.clone();
}

/// What a run did, one line per fix, plus what is still not in place.
#[derive(Default, Debug)]
pub struct Report {
    pub lines: Vec<String>,
    pub missing: Vec<&'static str>,
}
impl Report {
    fn note(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}

pub fn module_sha256() -> String {
    crate::update::package::hex(&Sha256::digest(MODULE))
}
/// A `name=value` string from a module's `.modinfo` section.
fn modinfo_field(module: &[u8], name: &str) -> Option<String> {
    let key = format!("{name}=");
    let start = module
        .windows(key.len())
        .position(|w| w == key.as_bytes())?
        + key.len();
    let len = module[start..].iter().position(|b| *b == 0)?;
    String::from_utf8(module[start..start + len].to_vec()).ok()
}
pub fn embedded_vermagic() -> Option<String> {
    modinfo_field(MODULE, "vermagic")
}
pub fn embedded_srcversion() -> Option<String> {
    modinfo_field(MODULE, "srcversion")
}

/// Bring the system into the fixed state. Never fails as a whole: every
/// problem is reported and leaves the affected part unchanged or switched off.
pub fn apply(o: &Options, host: &impl Host) -> Report {
    let mut r = Report::default();
    let mut state = State::load(o);
    let mut saved = state.clone();
    match eligible(o, host, &state) {
        Eligible::Yes => {
            let module = if let Some(reason) = state.failed_this_boot(o) {
                r.note(format!(
                    "HEVC decoder module: not installed (failed earlier in this boot: {reason}; \
                     tried again after the next reboot or with system-fixes enable-module)"
                ));
                false
            } else {
                match install_module(o, host) {
                    Ok(changed) => {
                        r.note(if changed {
                            "HEVC decoder module: installed (active after the next reboot)"
                        } else {
                            "HEVC decoder module: in place"
                        });
                        state.install_failed = None;
                        true
                    }
                    Err(e) => {
                        r.note(format!("HEVC decoder module: not installed ({e:#})"));
                        state.install_failed = boot_id(o).map(|boot_id| Failed {
                            module_sha256: module_sha256(),
                            boot_id,
                            reason: format!("{e:#}"),
                        });
                        false
                    }
                }
            };
            configure(o, module, &mut r);
            if !module {
                remove_module(o, host, &mut r);
            }
            hold(o, host, &mut state, module, &mut r);
            // Record the hold now: the module check below can take up to
            // 30 s, and a run killed before the save would forget which
            // packages are ours to release.
            save_state(o, &state, &mut saved, &mut r);
            if module && o.live {
                check_loaded(o, host, &mut state, &mut r);
            }
        }
        Eligible::No(reason) => {
            r.note(format!("HEVC decoder module: skipped ({reason})"));
            // Overlay line first, so no boot sees the line without its file.
            configure(o, false, &mut r);
            remove_module(o, host, &mut r);
            hold(o, host, &mut state, false, &mut r);
        }
        Eligible::Unknown(reason) => {
            // A check that could not run says nothing about the module:
            // leave whatever is installed as it is and retry next start.
            r.note(format!("HEVC decoder module: left as it is ({reason})"));
            // The overlay line only with its file: the firmware skips a
            // missing overlay and the memory pool would stay "pending".
            configure(
                o,
                o.installed_module().is_file() && o.overlay_file().is_file(),
                &mut r,
            );
        }
    }
    save_state(o, &state, &mut saved, &mut r);
    if !o.installed_module().is_file() {
        r.missing.push("module");
    }
    r
}

enum Eligible {
    Yes,
    No(String),
    /// A check could not run (a command failed).
    Unknown(String),
}
fn eligible(o: &Options, host: &impl Host, state: &State) -> Eligible {
    use Eligible::*;
    if let Some(reason) = state.disabled() {
        return No(format!("turned off: {reason}"));
    }
    if o.kernel_release != KERNEL_RELEASE {
        return No(format!(
            "kernel {} is not {KERNEL_RELEASE}, the release it was built for",
            o.kernel_release
        ));
    }
    let Some(vermagic) = embedded_vermagic().map(|v| v.trim().to_owned()) else {
        return No("embedded module has no vermagic".into());
    };
    if vermagic.split(' ').next() != Some(KERNEL_RELEASE) {
        return No(format!("embedded module is for {vermagic}"));
    }
    let admindir = o.root.join("var/lib/dpkg");
    let package = match host.run(
        "dpkg-query",
        &[
            &format!("--admindir={}", admindir.display()),
            "-W",
            "-f=${Version}",
            &format!("linux-image-{KERNEL_RELEASE}"),
        ],
    ) {
        Ok(p) => p,
        Err(e) => return Unknown(format!("cannot read the kernel package version: {e:#}")),
    };
    if package.trim() != KERNEL_PACKAGE {
        return No(format!(
            "kernel package {} is not {KERNEL_PACKAGE}",
            package.trim()
        ));
    }
    let Some(stock) = ["", ".xz", ".zst", ".gz"]
        .iter()
        .map(|ext| o.modules().join(format!("{STOCK_MODULE}{ext}")))
        .find(|p| p.is_file())
    else {
        return No("stock rpi-hevc-dec module not found".into());
    };
    let stock_magic = match host.run("modinfo", &["-F", "vermagic", &stock.display().to_string()]) {
        Ok(m) => m,
        Err(e) => return Unknown(format!("cannot read the stock module: {e:#}")),
    };
    if stock_magic.trim() != vermagic {
        return No(format!(
            "stock module vermagic '{}' differs from '{vermagic}'",
            stock_magic.trim()
        ));
    }
    Yes
}

/// The module and `depmod`, then the overlay file (only once the module is
/// known to be chosen, so a failing install never writes the boot
/// partition); the `config.txt` line follows only after this succeeded.
/// Returns whether anything changed.
fn install_module(o: &Options, host: &impl Host) -> Result<bool> {
    let mut changed = false;
    let target = o.installed_module();
    if fs::read(&target).ok().as_deref() != Some(MODULE) {
        fs::create_dir_all(target.parent().unwrap())?;
        replace_file(&target, MODULE, || Ok(())).context("module file")?;
        changed = true;
    }
    let listed = fs::read_to_string(o.modules().join("modules.dep"))
        .is_ok_and(|d| d.contains(&format!("{MODULE_DIR}/{MODULE_FILE}:")));
    let result = (|| -> Result<()> {
        if changed || !listed {
            depmod(o, host)?;
            changed = true;
        }
        let chosen = host.run(
            "modinfo",
            &[
                "-b",
                &o.root.display().to_string(),
                "-k",
                KERNEL_RELEASE,
                "-n",
                "rpi_hevc_dec",
            ],
        )?;
        if !chosen
            .trim()
            .ends_with(&format!("{MODULE_DIR}/{MODULE_FILE}"))
        {
            bail!("modprobe would load {} instead", chosen.trim());
        }
        let overlay = o.overlay_file();
        if fs::read(&overlay).ok().as_deref() != Some(OVERLAY) {
            fs::create_dir_all(overlay.parent().unwrap())?;
            replace_file(&overlay, OVERLAY, || Ok(())).context("overlay file")?;
            changed = true;
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&target);
        let _ = depmod(o, host);
        return Err(e);
    }
    Ok(changed)
}

fn depmod(o: &Options, host: &impl Host) -> Result<()> {
    host.run(
        "depmod",
        &["-b", &o.root.display().to_string(), KERNEL_RELEASE],
    )?;
    Ok(())
}

fn remove_module(o: &Options, host: &impl Host, r: &mut Report) {
    let target = o.installed_module();
    if target.exists() {
        match fs::remove_file(&target)
            .map_err(anyhow::Error::from)
            .and_then(|()| {
                let _ = fs::remove_dir(target.parent().unwrap());
                depmod(o, host)
            }) {
            Ok(()) => r.note(
                "HEVC decoder module: removed, the stock module is used after the next reboot",
            ),
            Err(e) => r.note(format!("HEVC decoder module: cannot remove ({e:#})")),
        }
    }
    let overlay = o.overlay_file();
    if overlay.exists()
        && let Err(e) = fs::remove_file(&overlay)
    {
        r.note(format!("Overlay file: cannot remove ({e})"));
    }
}

/// Every installed kernel image/header/kbuild package, versioned and meta:
/// an upgrade of the meta packages would pull in a new kernel, an upgrade of
/// the versioned ones could rebuild the same release with other symbol CRCs.
fn kernel_packages(o: &Options, host: &impl Host) -> Result<Vec<String>> {
    let admindir = o.root.join("var/lib/dpkg");
    let out = host.run(
        "dpkg-query",
        &[
            &format!("--admindir={}", admindir.display()),
            "-W",
            "-f=${db:Status-Status} ${Package}\n",
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|l| l.strip_prefix("installed "))
        .filter(|p| {
            ["linux-image-", "linux-headers-", "linux-kbuild-"]
                .iter()
                .any(|prefix| p.starts_with(prefix))
        })
        .map(str::to_owned)
        .collect())
}

fn hold(o: &Options, host: &impl Host, state: &mut State, module: bool, r: &mut Report) {
    if !o.packages {
        return;
    }
    let held: Vec<String> = match host.run("apt-mark", &["showhold"]) {
        Ok(out) => out.lines().map(str::to_owned).collect(),
        Err(e) => {
            r.note(format!("Kernel hold: cannot read ({e:#})"));
            return;
        }
    };
    if module {
        let wanted = match kernel_packages(o, host) {
            Ok(p) => p,
            Err(e) => {
                r.note(format!("Kernel hold: cannot list kernel packages ({e:#})"));
                return;
            }
        };
        let missing: Vec<&str> = wanted
            .iter()
            .filter(|p| !held.contains(p))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            let mut args = vec!["hold"];
            args.extend(&missing);
            match host.run("apt-mark", &args) {
                Ok(_) => r.note(format!("Kernel hold: {}", missing.join(" "))),
                Err(e) => {
                    r.note(format!("Kernel hold: failed ({e:#})"));
                    return;
                }
            }
        }
        for p in missing {
            if !state.held.iter().any(|h| h == p) {
                state.held.push(p.to_owned());
            }
        }
    } else if !state.held.is_empty() {
        let ours: Vec<&str> = state
            .held
            .iter()
            .filter(|p| held.contains(p))
            .map(String::as_str)
            .collect();
        if !ours.is_empty() {
            let mut args = vec!["unhold"];
            args.extend(&ours);
            if let Err(e) = host.run("apt-mark", &args) {
                r.note(format!("Kernel hold: cannot release ({e:#})"));
                return;
            }
            r.note(format!("Kernel hold released: {}", ours.join(" ")));
        }
        state.held.clear();
    }
}

/// After a boot with the module in place the decoder module must be loaded.
/// If it is not and cannot be, switch this module build off and fall back to
/// the stock module, so HEVC hardware decoding is never lost.
fn check_loaded(o: &Options, host: &impl Host, state: &mut State, r: &mut Report) {
    if o.kernel_release != KERNEL_RELEASE
        || o.root.join("sys/module/rpi_hevc_dec").exists()
        || !o.root.join(DECODER_NODE).exists()
    {
        return;
    }
    let Err(e) = host.run("modprobe", &["rpi_hevc_dec"]) else {
        r.note("HEVC decoder module: was not loaded, loaded it now");
        return;
    };
    r.note(format!(
        "HEVC decoder module: does not load ({e:#}); switching it off and using the stock module"
    ));
    state.module_disabled = Some(Disabled {
        module_sha256: module_sha256(),
        reason: format!("failed to load: {e:#}"),
    });
    configure(o, false, r);
    remove_module(o, host, r);
    hold(o, host, state, false, r);
    if let Err(e) = host.run("modprobe", &["rpi_hevc_dec"]) {
        r.note(format!(
            "Stock HEVC decoder module: does not load either ({e:#})"
        ));
    }
}

fn configure(o: &Options, overlay: bool, r: &mut Report) {
    let path = o.boot().join("config.txt");
    let Some(old) = read_config(&path, r) else {
        r.missing.push("config");
        return;
    };
    let fan_pin_taken = settings_pins(o).contains(&u64::from(FAN_GPIO));
    let edit = match edit_config(&old, overlay, fan_pin_taken) {
        Ok(e) => e,
        Err(reason) => {
            r.note(format!(
                "config.txt: skipped, unexpected content ({reason})"
            ));
            r.missing.push("config");
            return;
        }
    };
    r.lines.extend(edit.notes);
    r.missing.extend(edit.missing);
    if edit.text.as_bytes() == old.as_slice() {
        return;
    }
    match write_config(&path, &old, edit.text.as_bytes(), || Ok(())) {
        Ok(()) => r.note("config.txt: updated (takes effect after the next reboot)"),
        Err(e) => {
            r.note(format!("config.txt: not changed ({e:#})"));
            r.missing.push("config");
        }
    }
}

/// The current `config.txt`. Restored from `config.txt.ungoggled-prev` when
/// it is gone, empty or contains NUL bytes (what a power cut during a write
/// on the FAT boot partition can leave; without it the Pi boots without vc4
/// and dwc2) and that backup is one this code would edit. A damaged file is
/// kept as `config.txt.ungoggled-damaged`. Any other unexpected shape
/// (too big, not UTF-8, a broken block) may be the owner's doing and is left
/// alone.
fn read_config(path: &Path, r: &mut Report) -> Option<Vec<u8>> {
    let read = fs::read(path);
    let lost = match &read {
        Ok(b) => b.is_empty() || b.contains(&0),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    };
    let prev = path.with_file_name("config.txt.ungoggled-prev");
    if lost && prev.is_file() {
        match restore_config(path, &prev, read.as_deref().ok()) {
            Ok(b) => {
                r.note(
                    "config.txt: was missing, empty or damaged; restored from config.txt.ungoggled-prev",
                );
                return Some(b);
            }
            Err(e) => r.note(format!(
                "config.txt: cannot restore from config.txt.ungoggled-prev ({e:#})"
            )),
        }
    }
    match read {
        Ok(b) => Some(b),
        Err(e) => {
            r.note(format!(
                "config.txt: skipped, cannot read {} ({e})",
                path.display()
            ));
            None
        }
    }
}
fn restore_config(path: &Path, prev: &Path, damaged: Option<&[u8]>) -> Result<Vec<u8>> {
    let backup = fs::read(prev)?;
    if let Err(e) = edit_config(&backup, false, false) {
        bail!("unexpected content in the backup ({e})");
    }
    if let Some(d) = damaged.filter(|d| !d.is_empty()) {
        replace_file(
            &path.with_file_name("config.txt.ungoggled-damaged"),
            d,
            || Ok(()),
        )
        .context("keeping the damaged file")?;
    }
    replace_file(path, &backup, || Ok(()))?;
    if fs::read(path)? != backup {
        bail!("read back differs");
    }
    Ok(backup)
}

/// GPIO pins the app's settings use. Read loosely (any `*_pin` number and
/// every button's `pin`), so a field this release does not know still
/// counts; a missing or damaged file is simply no pins. A field missing
/// from the file means the app's default, and no default is GPIO 14.
fn settings_pins(o: &Options) -> Vec<u64> {
    let Some(serde_json::Value::Object(s)) = fs::read(o.root.join(SETTINGS))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    else {
        return Vec::new();
    };
    let mut pins: Vec<u64> = s
        .iter()
        .filter(|(k, _)| k.ends_with("_pin"))
        .filter_map(|(_, v)| v.as_u64())
        .collect();
    if let Some(buttons) = s.get("buttons").and_then(|b| b.as_array()) {
        pins.extend(
            buttons
                .iter()
                .filter_map(|b| b.get("pin").and_then(serde_json::Value::as_u64)),
        );
    }
    pins
}

/// Keep the file from before the first change and the one before the latest
/// change, then replace it atomically.
fn write_config(
    path: &Path,
    old: &[u8],
    new: &[u8],
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let backup = |suffix: &str| path.with_file_name(format!("config.txt.{suffix}"));
    if !backup("ungoggled-orig").exists() {
        replace_file(&backup("ungoggled-orig"), old, || Ok(())).context("backup")?;
    }
    replace_file(&backup("ungoggled-prev"), old, || Ok(())).context("backup")?;
    replace_file(path, new, before_rename)?;
    if fs::read(path)? != new {
        bail!("read back differs");
    }
    Ok(())
}

/// Write a sibling temporary file, fsync it, rename it over `path` and fsync
/// the directory. The previous content stays complete until the rename.
fn replace_file(
    path: &Path,
    bytes: &[u8],
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let name = path.file_name().context("no file name")?.to_string_lossy();
    let temp = path.with_file_name(format!(".{name}.ungoggled-tmp"));
    let result = (|| -> Result<()> {
        let mut f = fs::File::create(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        before_rename()?;
        fs::rename(&temp, path)?;
        // vfat accepts the directory fsync; it flushes the renamed entry.
        fs::File::open(path.parent().unwrap())?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

struct Edit {
    text: String,
    notes: Vec<String>,
    missing: Vec<&'static str>,
}
/// One `config.txt` line, its section filter and whether it is ours.
struct Line<'a> {
    text: &'a str,
    section: String,
    managed: bool,
}
impl Line<'_> {
    /// Applies on a Pi 4: no filter, `[all]` or `[pi4]`.
    fn applies(&self) -> bool {
        matches!(self.section.as_str(), "" | "all" | "pi4")
    }
    fn setting(&self) -> Option<(&str, &str)> {
        let t = self.text.trim();
        if t.starts_with('#') || t.starts_with('[') {
            return None;
        }
        t.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
    }
}
fn lines(text: &str) -> Result<Vec<Line<'_>>, String> {
    let mut section = String::new();
    let mut managed = false;
    let mut seen = false;
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let t = line.trim();
        if t == BEGIN {
            if managed || seen {
                return Err("ungoggled block twice".into());
            }
            managed = true;
            seen = true;
        } else if t == END {
            if !managed {
                return Err("ungoggled block end without start".into());
            }
            out.push(Line {
                text: line,
                section: section.clone(),
                managed: true,
            });
            managed = false;
            continue;
        } else if let Some(filter) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = filter.trim().to_ascii_lowercase();
        }
        out.push(Line {
            text: line,
            section: section.clone(),
            managed,
        });
    }
    if managed {
        return Err("ungoggled block not closed".into());
    }
    Ok(out)
}
/// Size in bytes of a vc4-kms-v3d CMA parameter, `None` if it is none.
fn cma_param(param: &str) -> Option<Result<u64, String>> {
    let p = param.trim();
    if let Some(v) = p.strip_prefix("cma-size=") {
        let n = match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
            Some(hex) => u64::from_str_radix(hex, 16),
            None => v.parse(),
        };
        return Some(n.map_err(|_| format!("unreadable '{p}'")));
    }
    if p == "cma-default" {
        return Some(Ok(0));
    }
    let mib = p.strip_prefix("cma-")?;
    Some(
        mib.parse::<u64>()
            .map(|m| m * 1024 * 1024)
            .map_err(|_| format!("unknown CMA parameter '{p}'")),
    )
}
/// The overlay a `dtoverlay=` value loads: its name, before any parameters.
fn overlay_name(value: &str) -> Option<&str> {
    value.split(',').next().map(str::trim)
}
/// Whether our `key=want` line belongs in the block: not when someone else
/// set the key outside it where it applies on a Pi 4 (the last such line
/// wins), which is then noted, and reported as missing if it differs.
fn plain_setting(
    parsed: &[Line<'_>],
    key: &'static str,
    want: &str,
    notes: &mut Vec<String>,
    missing: &mut Vec<&'static str>,
) -> bool {
    let theirs = parsed
        .iter()
        .filter(|l| !l.managed && l.applies())
        .filter_map(|l| l.setting())
        .filter(|(k, _)| *k == key)
        .map(|(_, v)| v)
        .next_back();
    match theirs {
        Some(v) if v == want => {
            notes.push(format!("{key}: already {v}"));
            false
        }
        Some(v) => {
            notes.push(format!("{key}: left at {v} (set outside ungoggled)"));
            missing.push(key);
            false
        }
        None => true,
    }
}
/// The [`NO_FAN`] comment, with any spacing and in any case.
fn is_no_fan_marker(line: &str) -> bool {
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    line.trim()
        .strip_prefix('#')
        .is_some_and(|rest| words(rest).eq_ignore_ascii_case(&words(&NO_FAN[1..])))
}
fn is_vc4(value: &str) -> bool {
    matches!(
        value.split(',').next().map(str::trim),
        Some("vc4-kms-v3d" | "vc4-kms-v3d-pi4")
    )
}

/// The new `config.txt`. `Err` means the file has a shape this code does not
/// edit; individual fixes it cannot apply safely are skipped with a note.
/// `fan_pin_taken`: the app's settings use the fan's GPIO pin.
fn edit_config(old: &[u8], overlay: bool, fan_pin_taken: bool) -> Result<Edit, String> {
    if old.is_empty() || old.len() > MAX_CONFIG || old.contains(&0) {
        return Err(format!("{} bytes", old.len()));
    }
    let text = std::str::from_utf8(old).map_err(|_| "not UTF-8")?;
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let parsed = lines(text)?;
    let mut notes = Vec::new();
    let mut missing = Vec::new();
    let user = || parsed.iter().filter(|l| !l.managed);
    // core_freq_min and hdmi_enable_4kp60: respect a value someone else set
    // where it applies.
    let core = plain_setting(
        &parsed,
        "core_freq_min",
        CORE_FREQ_MIN,
        &mut notes,
        &mut missing,
    );
    let hdmi_4kp60 = plain_setting(
        &parsed,
        "hdmi_enable_4kp60",
        HDMI_4KP60,
        &mut notes,
        &mut missing,
    );
    // Never together with an overclocked HEVC block or a changed voltage: that
    // combination killed the VideoCore firmware (no HDMI on either port) on
    // the test Pi. A config.txt with such a line keeps the owner's tuning and
    // does not get 4Kp60 from us; so does one that gets it later, because the
    // line is then left out of the rebuilt block.
    let hdmi_4kp60 = hdmi_4kp60
        && {
            let clash = user()
                .filter(|l| l.applies())
                .filter_map(|l| l.setting())
                .find(|(k, v)| match *k {
                    // 600 or more, or anything that is not a plain number.
                    "hevc_freq" => !v.trim().parse::<u32>().is_ok_and(|f| f < 600),
                    "over_voltage" | "over_voltage_delta" => v.trim() != "0",
                    _ => false,
                });
            match clash {
                Some((k, v)) => {
                    notes.push(format!(
                    "hdmi_enable_4kp60: not set ({k}={v} is in config.txt; together they stop the HDMI output)"
                ));
                    missing.push("hdmi_enable_4kp60");
                    false
                }
                None => true,
            }
        };
    // The fan: any gpio-fan overlay outside our block (what raspi-config
    // wrote, say) is the owner's choice of pin and temperature and stays.
    // Our line also stays out when GPIO 14 has another user: the app's own
    // settings, the serial console (`enable_uart=1`), or the owner's opt-out.
    let fan = {
        let elsewhere = user()
            .filter(|l| l.applies())
            .filter_map(|l| l.setting())
            .any(|(k, v)| k == "dtoverlay" && overlay_name(v) == Some(FAN_OVERLAY));
        let opted_out = user().any(|l| is_no_fan_marker(l.text));
        let uart = user()
            .filter(|l| l.applies())
            .filter_map(|l| l.setting())
            .filter(|(k, _)| *k == "enable_uart")
            .map(|(_, v)| v)
            .next_back()
            == Some("1");
        let skipped = if elsewhere {
            Some("already listed outside the ungoggled block".to_owned())
        } else if opted_out {
            Some(format!("skipped: '{NO_FAN}' is in config.txt"))
        } else if fan_pin_taken {
            Some(format!("skipped: GPIO {FAN_GPIO} is used by the settings"))
        } else if uart {
            Some(format!(
                "skipped: enable_uart=1 puts the serial console on GPIO {FAN_GPIO}"
            ))
        } else {
            None
        };
        if let Some(why) = &skipped {
            notes.push(format!("{FAN_OVERLAY}: {why}"));
        }
        skipped.is_none()
    };
    // The overlay line exists only together with the patched module.
    let overlay = overlay && {
        let elsewhere = user().filter_map(|l| l.setting()).any(|(k, v)| {
            k == "dtoverlay" && v.split(',').next().map(str::trim) == Some(OVERLAY_NAME)
        });
        if elsewhere {
            notes.push(format!(
                "{OVERLAY_NAME}: already listed outside the ungoggled block"
            ));
        }
        !elsewhere
    };
    // CMA pool: only the single vc4-kms-v3d line, and only upwards.
    let mut replaced: Option<(usize, String)> = None;
    let vc4: Vec<usize> = parsed
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.setting()
                .is_some_and(|(k, v)| k == "dtoverlay" && is_vc4(v))
        })
        .map(|(i, _)| i)
        .collect();
    let cma_dtparam = parsed
        .iter()
        .filter_map(|l| l.setting())
        .any(|(k, v)| k == "dtparam" && v.contains("cma"));
    let cma_note = match vc4.as_slice() {
        _ if cma_dtparam => Err("a dtparam line sets CMA".to_owned()),
        [] => Err("no vc4-kms-v3d overlay line".to_owned()),
        [i] if parsed[*i].managed || !parsed[*i].applies() => Err(format!(
            "vc4-kms-v3d line is in section [{}]",
            parsed[*i].section
        )),
        [i] => {
            let line = parsed[*i].text;
            let (key, value) = line.split_once('=').unwrap();
            let params: Vec<&str> = value.split(',').collect();
            let cma: Vec<(usize, Result<u64, String>)> = params
                .iter()
                .enumerate()
                .filter_map(|(n, p)| cma_param(p).map(|c| (n, c)))
                .collect();
            let target = format!("cma-size={CMA_SIZE:#x}");
            match cma.as_slice() {
                [] => {
                    replaced = Some((*i, format!("{}={},{target}", key, value.trim_end())));
                    Ok(format!("CMA size: set to {} MiB", CMA_SIZE >> 20))
                }
                [(_, Err(e))] => Err(e.clone()),
                [(_, Ok(size))] if *size >= CMA_SIZE => {
                    Ok(format!("CMA size: already {} MiB", size >> 20))
                }
                [(n, Ok(size))] => {
                    let mut p = params.clone();
                    let old = p[*n];
                    let new = format!("{target}{}", &old[old.trim_end().len()..]);
                    p[*n] = &new;
                    replaced = Some((*i, format!("{}={}", key, p.join(","))));
                    Ok(format!(
                        "CMA size: raised from {} to {} MiB",
                        size >> 20,
                        CMA_SIZE >> 20
                    ))
                }
                _ => Err("several CMA parameters".to_owned()),
            }
        }
        _ => Err("several vc4-kms-v3d lines".to_owned()),
    };
    match cma_note {
        Ok(n) => notes.push(n),
        Err(e) => {
            notes.push(format!("CMA size: left unchanged ({e})"));
            missing.push("cma");
        }
    }
    let mut block = Vec::new();
    if core {
        block.push(format!("core_freq_min={CORE_FREQ_MIN}"));
        notes.push(format!("core_freq_min: {CORE_FREQ_MIN}"));
    }
    if hdmi_4kp60 {
        block.push(format!("hdmi_enable_4kp60={HDMI_4KP60}"));
        notes.push(format!("hdmi_enable_4kp60: {HDMI_4KP60}"));
    }
    if fan {
        let line = format!("dtoverlay={FAN_OVERLAY},gpiopin={FAN_GPIO},temp={FAN_ON_MILLI_C}");
        notes.push(format!("{line}: listed"));
        block.push(line);
    }
    if overlay {
        block.push(format!("dtoverlay={OVERLAY_NAME}"));
        notes.push(format!("dtoverlay={OVERLAY_NAME}: listed"));
    }
    // A `dtparam=` line is a parameter of the most recent `dtoverlay=` line;
    // an empty one ends that scope, so lines added below the block (by hand
    // or by raspi-config) still reach the base device tree.
    if block.iter().any(|l| l.starts_with("dtoverlay=")) {
        block.push("dtoverlay=".to_owned());
    }
    let block: Vec<String> = if block.is_empty() {
        Vec::new()
    } else {
        [BEGIN.to_owned(), "[all]".to_owned()]
            .into_iter()
            .chain(block)
            .chain([END.to_owned()])
            .collect()
    };
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    for (i, l) in parsed.iter().enumerate() {
        if l.managed {
            if !placed {
                out.extend(block.iter().cloned());
                placed = true;
            }
            continue;
        }
        match &replaced {
            Some((n, new)) if *n == i => out.push(new.clone()),
            _ => out.push(l.text.to_owned()),
        }
    }
    if !placed && !block.is_empty() {
        // `text` ends with a newline in a normal file, giving a final "".
        let ends_with_newline = out.last().is_some_and(|l| l.is_empty());
        if ends_with_newline {
            out.pop();
        }
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.extend(block);
        out.push(String::new());
    }
    Ok(Edit {
        text: out.join(eol),
        notes,
        missing,
    })
}

/// Why a reboot is still needed: what `config.txt` and the installed module
/// ask for but the running system does not have yet. Empty if nothing is
/// pending or nothing can be compared (e.g. not a Pi).
pub fn pending(root: &Path, host: &impl Host) -> Vec<String> {
    let mut reasons = Vec::new();
    let Ok(old) = fs::read(root.join(BOOT).join("config.txt")) else {
        return reasons;
    };
    let text = String::from_utf8_lossy(&old);
    let Ok(parsed) = lines(&text) else {
        return reasons;
    };
    let effective = |key: &str| {
        parsed
            .iter()
            .filter(|l| l.applies())
            .filter_map(|l| l.setting())
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.to_owned())
            .collect::<Vec<_>>()
    };
    let core = effective("core_freq_min");
    if let Some(want) = core.last()
        && let Ok(out) = host.run("vcgencmd", &["get_config", "core_freq_min"])
        && !out
            .trim()
            .strip_prefix("core_freq_min=")
            .is_some_and(|v| same_value(want, v))
    {
        reasons.push(format!("core_freq_min={want} not active ({})", out.trim()));
    }
    if let Some(want) = effective("hdmi_enable_4kp60").last()
        && let Ok(out) = host.run("vcgencmd", &["get_config", "hdmi_enable_4kp60"])
        && !out
            .trim()
            .strip_prefix("hdmi_enable_4kp60=")
            .is_some_and(|v| same_value(want, v))
    {
        reasons.push(format!(
            "hdmi_enable_4kp60={want} not active ({})",
            out.trim()
        ));
    }
    let overlays = effective("dtoverlay");
    // The fan overlay shows up in the device tree as a `gpio-fan` node.
    let device_tree = root.join("proc/device-tree");
    if device_tree.is_dir()
        && overlays
            .iter()
            .any(|v| overlay_name(v) == Some(FAN_OVERLAY))
        && !fs::read_dir(&device_tree)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(FAN_OVERLAY))
    {
        reasons.push("fan control appears after the next boot".to_owned());
    }
    let cma = overlays
        .iter()
        .filter(|v| is_vc4(v))
        .flat_map(|v| v.split(',').filter_map(cma_param).collect::<Vec<_>>())
        .filter_map(Result::ok)
        .next_back();
    if let Some(want) = cma.filter(|s| *s > 0)
        && let Ok(bytes) = fs::read(root.join("proc/device-tree/reserved-memory/linux,cma/size"))
    {
        let running = bytes.iter().fold(0_u64, |a, b| (a << 8) | u64::from(*b));
        if running != want {
            reasons.push(format!(
                "CMA pool is {} MiB, configured {}",
                running >> 20,
                want >> 20
            ));
        }
    }
    let decoder = root.join(DECODER_NODE);
    if decoder.exists() {
        let configured = overlays
            .iter()
            .any(|v| v.split(',').next().map(str::trim) == Some(OVERLAY_NAME));
        let running = decoder.join("memory-region").exists();
        if configured != running {
            reasons.push(format!(
                "decoder memory pool {} after the next boot",
                if configured { "appears" } else { "goes away" }
            ));
        }
    }
    let installed = root
        .join("lib/modules")
        .join(KERNEL_RELEASE)
        .join(MODULE_DIR)
        .join(MODULE_FILE)
        .is_file();
    let running_release = fs::read_to_string(root.join("proc/sys/kernel/osrelease"))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    if running_release == KERNEL_RELEASE
        && let Ok(loaded) = fs::read_to_string(root.join("sys/module/rpi_hevc_dec/srcversion"))
    {
        let ours = embedded_srcversion().is_some_and(|s| s == loaded.trim());
        if installed != ours {
            reasons.push(format!(
                "HEVC decoder module: {} module loaded",
                if ours { "patched" } else { "stock" }
            ));
        }
    }
    reasons
}

/// Whether the firmware's answer is the configured value: compared as
/// numbers where both are numbers (`0500`, `0x1f4`), as text otherwise.
fn same_value(want: &str, running: &str) -> bool {
    let number = |s: &str| {
        let s = s.trim();
        match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => s.parse::<u64>().ok(),
        }
    };
    match (number(want), number(running)) {
        (Some(a), Some(b)) => a == b,
        _ => want.trim() == running.trim(),
    }
}

/// Size and modification time of what [`pending`] compares on the
/// configured side, plus whether a decoder module is loaded: when none of
/// it changes, neither does the answer.
fn pending_inputs(root: &Path) -> Vec<Option<(std::time::SystemTime, u64)>> {
    let modules = root.join("lib/modules").join(KERNEL_RELEASE);
    [
        root.join(BOOT).join("config.txt"),
        root.join(BOOT)
            .join("overlays")
            .join(format!("{OVERLAY_NAME}.dtbo")),
        modules.join(MODULE_DIR).join(MODULE_FILE),
        root.join("sys/module/rpi_hevc_dec/srcversion"),
    ]
    .iter()
    .map(|p| {
        let m = fs::metadata(p).ok()?;
        Some((m.modified().ok()?, m.len()))
    })
    .collect()
}

static PENDING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// Work out in the background whether this boot still lacks fixes that are
/// already configured. They are applied before the service starts and act
/// only after a reboot, but `system-fixes remove-module`/`enable-module` run
/// by hand change them while it runs; so the inputs are looked at once a
/// minute and the answer is worked out again when they changed.
pub fn watch() {
    std::thread::spawn(|| {
        let root = Path::new("/");
        let mut seen = None;
        loop {
            let inputs = pending_inputs(root);
            if seen.as_ref() != Some(&inputs) {
                let reasons = pending(root, &System);
                let mut current = PENDING.lock().unwrap_or_else(|e| e.into_inner());
                if !reasons.is_empty() && *current != reasons {
                    eprintln!(
                        "Reboot needed to finish system fixes: {}",
                        reasons.join("; ")
                    );
                }
                *current = reasons;
                seen = Some(inputs);
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
}
pub fn reboot_required() -> bool {
    !PENDING.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

/// `ungoggled system-fixes ...`
pub fn command(
    action: &str,
    root: &Path,
    kernel_release: Option<String>,
    offline: bool,
    strict: bool,
) -> Result<()> {
    let o = Options::system(root, kernel_release, offline)?;
    match action {
        "status" => {
            let reasons = pending(root, &System);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "kernel_release": o.kernel_release,
                    "module_built_for": KERNEL_RELEASE,
                    "module_srcversion": embedded_srcversion(),
                    "module_installed": o.installed_module().is_file(),
                    "module_disabled": State::load(&o).module_disabled,
                    "reboot_required": !reasons.is_empty(),
                    "reasons": reasons,
                }))?
            );
            return Ok(());
        }
        "remove-module" | "enable-module" => {
            let mut state = State::load(&o);
            state.module_disabled = (action == "remove-module").then(|| Disabled {
                module_sha256: module_sha256(),
                reason: "removed by hand (system-fixes enable-module turns it back on)".into(),
            });
            // Either way the next install attempt starts from scratch.
            state.install_failed = None;
            state.save(&o)?;
        }
        "apply" => {}
        other => bail!("unknown action {other}"),
    }
    let report = apply(&o, &System);
    for line in &report.lines {
        println!("system-fixes: {line}");
    }
    if strict && !report.missing.is_empty() {
        bail!("not applied: {}", report.missing.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
