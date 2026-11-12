//! Temporary root directories and a fake command runner; nothing here touches
//! the real system.
use super::*;
use crate::update::tests::Fixture;
use std::cell::RefCell;

/// `config.txt` as shipped by the 0.3.42 flash image.
const IMAGE_CONFIG: &str = "# For more options and information see
# http://rptl.io/configtxt
dtparam=audio=on
camera_auto_detect=1
display_auto_detect=1
auto_initramfs=1

# Enable DRM VC4 V3D driver
dtoverlay=vc4-kms-v3d,cma-size=0x28000000
max_framebuffers=2
disable_fw_kms_setup=1
arm_64bit=1
disable_overscan=1
arm_boost=1

[cm4]
otg_mode=1

[cm5]
dtoverlay=dwc2,dr_mode=host

[pi5]
dtoverlay=nospi10

[all]

[all]
# ungoggled USB accessory controller
dtoverlay=dwc2,dr_mode=peripheral
";
const STOCK_STOCK: &str = "dtparam=audio=on
dtoverlay=vc4-kms-v3d
max_framebuffers=2

[cm5]
dtoverlay=dwc2,dr_mode=host

[all]
dtoverlay=dwc2,dr_mode=peripheral
";
const PACKAGES: &str = "installed linux-base-rpi-v8
installed linux-headers-6.18.50+rpt-common-rpi
installed linux-headers-6.18.50+rpt-rpi-v8
installed linux-headers-rpi-v8
installed linux-image-6.18.50+rpt-rpi-2712
installed linux-image-6.18.50+rpt-rpi-v8
installed linux-image-rpi-v8
installed linux-kbuild-6.18.50+rpt
installed linux-libc-dev
config-files linux-image-6.12.1+rpt-rpi-v8
installed mpv
";

struct Fake {
    root: PathBuf,
    calls: RefCell<Vec<String>>,
    package: String,
    stock_vermagic: String,
    /// What `modinfo -n` reports; `None`: the path depmod would pick.
    chosen: Option<String>,
    modprobe_ok: bool,
    dpkg_ok: bool,
    held: RefCell<Vec<String>>,
    vcgencmd: String,
    /// The state file as it was on disk when `modprobe` last ran.
    state_at_modprobe: RefCell<Option<String>>,
}
impl Fake {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            calls: RefCell::default(),
            package: KERNEL_PACKAGE.into(),
            stock_vermagic: embedded_vermagic().unwrap(),
            chosen: None,
            modprobe_ok: true,
            dpkg_ok: true,
            held: RefCell::default(),
            vcgencmd: "core_freq_min=0\nhdmi_enable_4kp60=0".into(),
            state_at_modprobe: RefCell::default(),
        }
    }
    fn calls(&self, program: &str) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .filter(|c| c.starts_with(&format!("{program} ")))
            .cloned()
            .collect()
    }
    fn clear(&self) {
        self.calls.borrow_mut().clear();
    }
}
impl Host for Fake {
    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        self.calls
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        let modules = self.root.join("lib/modules").join(KERNEL_RELEASE);
        let ours = format!("{MODULE_DIR}/{MODULE_FILE}");
        if program == "modprobe" {
            *self.state_at_modprobe.borrow_mut() = fs::read_to_string(self.root.join(STATE)).ok();
        }
        match program {
            "dpkg-query" if !self.dpkg_ok => bail!("dpkg-query: database locked"),
            "dpkg-query" if args.contains(&"-f=${Version}") => Ok(self.package.clone()),
            "dpkg-query" => Ok(PACKAGES.into()),
            "modinfo" if args[0] == "-F" => Ok(format!("{}\n", self.stock_vermagic)),
            "modinfo" => Ok(match &self.chosen {
                Some(c) => c.clone(),
                None if modules.join(&ours).is_file() => {
                    format!("{}\n", modules.join(&ours).display())
                }
                None => format!("{}\n", modules.join(STOCK_MODULE).display()),
            }),
            "depmod" => {
                let dep = if modules.join(&ours).is_file() {
                    format!("{ours}: kernel/drivers/media/v4l2-core/v4l2-mem2mem.ko.xz\n")
                } else {
                    format!(
                        "{STOCK_MODULE}.xz: kernel/drivers/media/v4l2-core/v4l2-mem2mem.ko.xz\n"
                    )
                };
                fs::write(modules.join("modules.dep"), dep)?;
                Ok(String::new())
            }
            "apt-mark" if args[0] == "showhold" => Ok(self
                .held
                .borrow()
                .iter()
                .map(|p| format!("{p}\n"))
                .collect()),
            "apt-mark" if args[0] == "hold" => {
                self.held
                    .borrow_mut()
                    .extend(args[1..].iter().map(|s| s.to_string()));
                Ok(String::new())
            }
            "apt-mark" if args[0] == "unhold" => {
                self.held
                    .borrow_mut()
                    .retain(|p| !args[1..].contains(&p.as_str()));
                Ok(String::new())
            }
            "modprobe" if self.modprobe_ok => Ok(String::new()),
            "modprobe" => {
                bail!("modprobe: ERROR: could not insert 'rpi_hevc_dec': Exec format error")
            }
            "vcgencmd" => {
                // `vcgencmd get_config KEY` answers with that key's line only.
                let key = format!("{}=", args.last().copied().unwrap_or_default());
                Ok(self
                    .vcgencmd
                    .lines()
                    .find(|l| l.starts_with(&key))
                    .map(|l| format!("{l}\n"))
                    .unwrap_or_default())
            }
            other => bail!("unexpected command {other}"),
        }
    }
}

struct Pi {
    _fixture: Fixture,
    root: PathBuf,
}
impl Pi {
    fn new(config: Option<&str>) -> Self {
        let fixture = Fixture::new();
        let root = fixture.0.join("root");
        fs::create_dir_all(root.join(BOOT).join("overlays")).unwrap();
        if let Some(c) = config {
            fs::write(root.join(BOOT).join("config.txt"), c).unwrap();
        }
        let stock = root
            .join("lib/modules")
            .join(KERNEL_RELEASE)
            .join(format!("{STOCK_MODULE}.xz"));
        fs::create_dir_all(stock.parent().unwrap()).unwrap();
        fs::write(&stock, b"stock module").unwrap();
        fs::create_dir_all(root.join("var/lib/dpkg")).unwrap();
        Self {
            _fixture: fixture,
            root,
        }
    }
    fn options(&self, release: &str) -> Options {
        Options {
            root: self.root.clone(),
            kernel_release: release.into(),
            live: false,
            packages: true,
        }
    }
    fn config(&self) -> String {
        fs::read_to_string(self.root.join(BOOT).join("config.txt")).unwrap()
    }
    fn module(&self) -> PathBuf {
        self.options(KERNEL_RELEASE).installed_module()
    }
    fn overlay(&self) -> PathBuf {
        self.options(KERNEL_RELEASE).overlay_file()
    }
    fn state(&self) -> State {
        State::load(&self.options(KERNEL_RELEASE))
    }
    fn leftovers(&self) -> Vec<String> {
        let mut found = Vec::new();
        for dir in [
            self.root.join(BOOT),
            self.module().parent().unwrap().to_path_buf(),
        ] {
            for e in fs::read_dir(dir).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.contains("ungoggled-tmp") {
                    found.push(n);
                }
            }
        }
        found
    }
}

const BLOCK: &str = "# >>> ungoggled system fixes (managed automatically, edits here are replaced)
[all]
core_freq_min=500
hdmi_enable_4kp60=1
dtoverlay=gpio-fan,gpiopin=14,temp=60000
dtoverlay=ungoggled-hevc-cma
dtoverlay=
# <<< ungoggled system fixes
";

#[test]
fn embedded_module_matches_the_pinned_kernel_and_build_info() {
    let vermagic = embedded_vermagic().unwrap();
    assert_eq!(vermagic.split(' ').next(), Some(KERNEL_RELEASE));
    let info: std::collections::HashMap<&str, &str> = BUILD_INFO
        .lines()
        .filter_map(|l| l.split_once('='))
        .collect();
    assert_eq!(info["kernel_release"], KERNEL_RELEASE);
    assert_eq!(info["kernel_package"], KERNEL_PACKAGE);
    assert_eq!(info["vermagic"], vermagic.trim());
    assert_eq!(info["srcversion"], embedded_srcversion().unwrap());
    assert_eq!(info["module_sha256"], module_sha256());
    assert_eq!(
        info["overlay_sha256"],
        crate::update::package::hex(&Sha256::digest(OVERLAY))
    );
    let patch = include_bytes!(
        "../../docs/upstream-patches/rpi-hevc-memory-region/0001-rpi-hevc-dec-memory-region.patch"
    );
    assert_eq!(
        info["patch_sha256"],
        crate::update::package::hex(&Sha256::digest(patch))
    );
    // A device-tree blob, compiled with symbols for the firmware's loader.
    assert_eq!(&OVERLAY[..4], &[0xd0, 0x0d, 0xfe, 0xed]);
}

#[test]
fn fresh_image_gets_every_fix_and_a_second_run_changes_nothing() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let host = Fake::new(&pi.root);
    let o = pi.options(KERNEL_RELEASE);
    let r = apply(&o, &host);
    assert!(r.missing.is_empty(), "{r:?}");
    assert_eq!(pi.config(), format!("{IMAGE_CONFIG}\n{BLOCK}"));
    assert_eq!(fs::read(pi.module()).unwrap(), MODULE);
    assert_eq!(fs::read(pi.overlay()).unwrap(), OVERLAY);
    assert_eq!(
        fs::read_to_string(pi.root.join(BOOT).join("config.txt.ungoggled-orig")).unwrap(),
        IMAGE_CONFIG
    );
    assert_eq!(host.calls("depmod").len(), 1);
    assert!(host.calls("depmod")[0].ends_with(KERNEL_RELEASE));
    let held = host.held.borrow().clone();
    assert_eq!(
        held,
        [
            "linux-headers-6.18.50+rpt-common-rpi",
            "linux-headers-6.18.50+rpt-rpi-v8",
            "linux-headers-rpi-v8",
            "linux-image-6.18.50+rpt-rpi-2712",
            "linux-image-6.18.50+rpt-rpi-v8",
            "linux-image-rpi-v8",
            "linux-kbuild-6.18.50+rpt",
        ]
    );
    assert_eq!(pi.state().held, held);
    assert!(r.lines.iter().any(|l| l == "CMA size: already 640 MiB"));
    assert!(pi.leftovers().is_empty());

    let config = pi.config();
    let modified = fs::metadata(pi.root.join(BOOT).join("config.txt"))
        .unwrap()
        .modified()
        .unwrap();
    host.clear();
    let r = apply(&o, &host);
    assert!(r.missing.is_empty(), "{r:?}");
    assert_eq!(pi.config(), config);
    assert_eq!(
        fs::metadata(pi.root.join(BOOT).join("config.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    assert!(host.calls("depmod").is_empty());
    assert_eq!(host.calls("apt-mark"), ["apt-mark showhold"]);
    assert!(
        !r.lines
            .iter()
            .any(|l| l.contains("updated") || l.contains("installed"))
    );
}

#[test]
fn stock_config_gets_the_cma_size_on_its_vc4_line() {
    let pi = Pi::new(Some(STOCK_STOCK));
    apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
    assert_eq!(
        pi.config(),
        format!(
            "{}\n{BLOCK}",
            STOCK_STOCK.replace(
                "dtoverlay=vc4-kms-v3d\n",
                "dtoverlay=vc4-kms-v3d,cma-size=0x28000000\n"
            )
        )
    );
}

#[test]
fn cma_size_is_only_raised_and_only_on_an_unambiguous_line() {
    let edit = |line: &str| {
        let e = edit_config(format!("a=1\n{line}\n").as_bytes(), false, false).unwrap();
        (e.text.lines().nth(1).unwrap().to_owned(), e.missing)
    };
    assert_eq!(
        edit("dtoverlay=vc4-kms-v3d,cma-512").0,
        "dtoverlay=vc4-kms-v3d,cma-size=0x28000000"
    );
    assert_eq!(
        edit("dtoverlay=vc4-kms-v3d,cma-size=0x10000000,noaudio").0,
        "dtoverlay=vc4-kms-v3d,cma-size=0x28000000,noaudio"
    );
    assert_eq!(
        edit("dtoverlay=vc4-kms-v3d-pi4,cma-default").0,
        "dtoverlay=vc4-kms-v3d-pi4,cma-size=0x28000000"
    );
    assert_eq!(
        edit("dtoverlay=vc4-kms-v3d,cma-size=0x30000000").0,
        "dtoverlay=vc4-kms-v3d,cma-size=0x30000000"
    );
    assert_eq!(
        edit("dtoverlay=vc4-kms-v3d,cma-size=805306368").1,
        Vec::<&str>::new()
    );
    for odd in [
        "dtoverlay=vc4-kms-v3d,cma-size=lots",
        "dtoverlay=vc4-kms-v3d,cma-256,cma-size=0x10000000",
        "dtoverlay=vc4-fkms-v3d",
        "dtoverlay=vc4-kms-v3d\ndtparam=cma-size=0x10000000",
        "dtoverlay=vc4-kms-v3d\n[all]\ndtoverlay=vc4-kms-v3d",
        "[pi3]\ndtoverlay=vc4-kms-v3d",
    ] {
        let (_, missing) = edit(odd);
        assert_eq!(missing, ["cma"], "{odd}");
        let e = edit_config(format!("a=1\n{odd}\n").as_bytes(), false, false).unwrap();
        assert!(e.text.contains(odd), "{odd} changed");
    }
}

#[test]
fn someone_elses_core_freq_min_is_respected() {
    let e = edit_config(
        b"dtoverlay=vc4-kms-v3d,cma-size=0x28000000\ncore_freq_min=600\n",
        false,
        false,
    )
    .unwrap();
    assert!(!e.text.contains("core_freq_min=500"));
    assert_eq!(e.missing, ["core_freq_min"]);
    let e = edit_config(
        b"dtoverlay=vc4-kms-v3d,cma-size=0x28000000\n[all]\ncore_freq_min=500\n",
        true,
        false,
    )
    .unwrap();
    assert_eq!(e.text.matches("core_freq_min").count(), 1);
    assert!(e.missing.is_empty());
    // Under a filter that does not match a Pi 4 it does not count.
    let e = edit_config(
        b"dtoverlay=vc4-kms-v3d,cma-size=0x28000000\n[pi5]\ncore_freq_min=600\n",
        false,
        false,
    )
    .unwrap();
    assert!(e.text.ends_with(
        "[all]\ncore_freq_min=500\nhdmi_enable_4kp60=1\ndtoverlay=gpio-fan,gpiopin=14,temp=60000\n\
                 dtoverlay=\n# <<< ungoggled system fixes\n"
    ));
}

#[test]
fn crlf_files_keep_crlf_and_files_without_final_newline_get_one() {
    let e = edit_config(b"dtoverlay=vc4-kms-v3d\r\narm_64bit=1\r\n", true, false).unwrap();
    assert!(e.text.ends_with("# <<< ungoggled system fixes\r\n"));
    assert!(!e.text.replace("\r\n", "").contains('\n'));
    let e = edit_config(b"dtoverlay=vc4-kms-v3d\narm_64bit=1", false, false).unwrap();
    assert_eq!(
        e.text,
        "dtoverlay=vc4-kms-v3d,cma-size=0x28000000\narm_64bit=1\n\n\
         # >>> ungoggled system fixes (managed automatically, edits here are replaced)\n[all]\ncore_freq_min=500\n\
         hdmi_enable_4kp60=1\ndtoverlay=gpio-fan,gpiopin=14,temp=60000\ndtoverlay=\n\
         # <<< ungoggled system fixes\n"
    );
    assert_eq!(
        edit_config(e.text.as_bytes(), false, false).unwrap().text,
        e.text
    );
}

#[test]
fn unexpected_config_is_left_alone_but_the_rest_still_runs() {
    for bad in [
        "".to_owned(),
        format!("a=1\n{BEGIN}\ncore_freq_min=500\n"),
        format!("a=1\n{END}\n"),
        format!("{BLOCK}{BLOCK}"),
        "a=\0\n".to_owned(),
        "x".repeat(MAX_CONFIG + 1),
    ] {
        let pi = Pi::new(Some(&bad));
        let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
        assert_eq!(pi.config(), bad);
        assert!(r.missing.contains(&"config"), "{bad:?}");
        assert!(pi.module().is_file());
        assert!(
            !pi.root
                .join(BOOT)
                .join("config.txt.ungoggled-orig")
                .exists()
        );
    }
    let pi = Pi::new(None);
    fs::write(pi.root.join(BOOT).join("config.txt"), [0xff, 0xfe, b'\n']).unwrap();
    let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
    assert!(r.missing.contains(&"config"));
}

#[test]
fn missing_config_is_skipped() {
    let pi = Pi::new(None);
    let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
    assert!(r.missing.contains(&"config"));
    assert!(
        r.lines
            .iter()
            .any(|l| l.starts_with("config.txt: skipped, cannot read"))
    );
    assert!(!pi.root.join(BOOT).join("config.txt").exists());
}

#[test]
fn read_only_boot_partition_is_skipped_without_damage() {
    if unsafe { libc::geteuid() } == 0 {
        return; // root ignores the permission bits this test relies on
    }
    use std::os::unix::fs::PermissionsExt;
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let boot = pi.root.join(BOOT);
    fs::set_permissions(&boot, fs::Permissions::from_mode(0o555)).unwrap();
    let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
    fs::set_permissions(&boot, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(pi.config(), IMAGE_CONFIG);
    assert!(r.missing.contains(&"config"), "{r:?}");
    assert!(pi.leftovers().is_empty());
}

#[test]
fn interrupted_write_keeps_the_old_file_and_a_stale_temp_file_is_harmless() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let path = pi.root.join(BOOT).join("config.txt");
    let err = write_config(&path, IMAGE_CONFIG.as_bytes(), b"new", || {
        bail!("power cut")
    })
    .unwrap_err();
    assert!(err.to_string().contains("power cut"));
    assert_eq!(pi.config(), IMAGE_CONFIG);
    assert!(pi.leftovers().is_empty());
    // A crash after the temp file was written but before the rename.
    fs::write(pi.root.join(BOOT).join(".config.txt.ungoggled-tmp"), "half").unwrap();
    let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
    assert!(r.missing.is_empty(), "{r:?}");
    assert_eq!(pi.config(), format!("{IMAGE_CONFIG}\n{BLOCK}"));
    assert!(pi.leftovers().is_empty());
}

#[test]
fn other_kernel_gets_config_only_and_loses_an_installed_module() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let host = Fake::new(&pi.root);
    let r = apply(&pi.options("6.18.51+rpt-rpi-v8"), &host);
    assert!(!pi.module().exists() && !pi.overlay().exists());
    assert!(pi.config().contains("core_freq_min=500"));
    assert!(!pi.config().contains("ungoggled-hevc-cma"));
    assert!(host.held.borrow().is_empty());
    assert!(r.lines[0].contains("is not 6.18.50+rpt-rpi-v8"), "{r:?}");
    assert_eq!(r.missing, ["module"]);

    apply(&pi.options(KERNEL_RELEASE), &host);
    assert!(pi.module().is_file() && pi.config().contains("ungoggled-hevc-cma"));
    assert!(!host.held.borrow().is_empty());
    host.clear();
    apply(&pi.options("6.18.51+rpt-rpi-v8"), &host);
    assert!(!pi.module().exists() && !pi.overlay().exists());
    assert!(!pi.config().contains("ungoggled-hevc-cma"));
    assert!(pi.config().contains("core_freq_min=500"));
    assert_eq!(host.calls("depmod").len(), 1);
    assert!(host.held.borrow().is_empty());
    assert!(pi.state().held.is_empty());
    // The stock module is never touched.
    assert!(
        pi.root
            .join("lib/modules")
            .join(KERNEL_RELEASE)
            .join(format!("{STOCK_MODULE}.xz"))
            .is_file()
    );
}

#[test]
fn vermagic_or_package_mismatch_installs_nothing() {
    for (vermagic, package) in [
        (
            "6.18.50+rpt-rpi-v8 SMP preempt mod_unload aarch64",
            KERNEL_PACKAGE,
        ),
        ("", KERNEL_PACKAGE),
        (embedded_vermagic().unwrap().as_str(), "1:6.18.50-1+rpt2"),
    ] {
        let pi = Pi::new(Some(IMAGE_CONFIG));
        let mut host = Fake::new(&pi.root);
        host.stock_vermagic = vermagic.into();
        host.package = package.into();
        let r = apply(&pi.options(KERNEL_RELEASE), &host);
        assert!(!pi.module().exists(), "{vermagic} {package}");
        assert!(!pi.config().contains("ungoggled-hevc-cma"));
        assert!(pi.config().contains("core_freq_min=500"));
        assert!(host.calls("depmod").is_empty());
        assert!(r.lines[0].contains("skipped"), "{r:?}");
    }
}

#[test]
fn a_check_that_cannot_run_leaves_the_module_as_it_is() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let mut host = Fake::new(&pi.root);
    apply(&pi.options(KERNEL_RELEASE), &host);
    let config = pi.config();
    host.dpkg_ok = false;
    host.clear();
    let r = apply(&pi.options(KERNEL_RELEASE), &host);
    assert!(pi.module().is_file());
    assert_eq!(pi.config(), config);
    assert!(host.calls("depmod").is_empty() && host.calls("apt-mark").is_empty());
    assert!(r.lines[0].contains("left as it is"), "{r:?}");
}

#[test]
fn depmod_choosing_another_module_rolls_the_module_back() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let mut host = Fake::new(&pi.root);
    host.chosen = Some("/lib/modules/x/extra/rpi-hevc-dec.ko\n".into());
    let r = apply(&pi.options(KERNEL_RELEASE), &host);
    assert!(!pi.module().exists());
    assert!(!pi.config().contains("ungoggled-hevc-cma"));
    assert!(r.lines[0].contains("not installed"), "{r:?}");
    assert!(host.held.borrow().is_empty());
}

#[test]
fn a_module_that_does_not_load_is_switched_off_for_good() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    fs::create_dir_all(pi.root.join(DECODER_NODE)).unwrap();
    let mut host = Fake::new(&pi.root);
    host.modprobe_ok = false;
    let mut o = pi.options(KERNEL_RELEASE);
    o.live = true;
    let r = apply(&o, &host);
    assert!(r.lines.iter().any(|l| l.contains("does not load")), "{r:?}");
    assert!(!pi.module().exists() && !pi.overlay().exists());
    assert!(!pi.config().contains("ungoggled-hevc-cma"));
    assert!(pi.config().contains("core_freq_min=500"));
    assert!(host.held.borrow().is_empty());
    assert_eq!(host.calls("modprobe").len(), 2);
    assert!(pi.state().disabled().unwrap().contains("Exec format error"));
    // Stays off on the next start instead of trying again.
    host.modprobe_ok = true;
    host.clear();
    let r = apply(&o, &host);
    assert!(!pi.module().exists());
    assert!(r.lines[0].contains("turned off"), "{r:?}");
    // A loaded decoder (the stock one) is never probed.
    let mut state = pi.state();
    state.module_disabled = None;
    state.save(&o).unwrap();
    fs::create_dir_all(pi.root.join("sys/module/rpi_hevc_dec")).unwrap();
    host.clear();
    apply(&o, &host);
    assert!(pi.module().is_file());
    assert!(host.calls("modprobe").is_empty());
}

#[test]
fn pending_compares_configuration_with_the_running_system() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let mut host = Fake::new(&pi.root);
    apply(&pi.options(KERNEL_RELEASE), &host);
    let write = |p: &str, b: &[u8]| {
        let p = pi.root.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    };
    write(
        "proc/sys/kernel/osrelease",
        format!("{KERNEL_RELEASE}\n").as_bytes(),
    );
    write(
        "proc/device-tree/reserved-memory/linux,cma/size",
        &[0x28, 0, 0, 0],
    );
    write(
        &format!("{DECODER_NODE}/compatible"),
        b"brcm,bcm2711-hevc-dec\0",
    );
    write(
        "sys/module/rpi_hevc_dec/srcversion",
        b"A8E8382C2538332363EC202\n",
    );
    // Just updated, not rebooted: clock floor, 4Kp60, fan, pool and module
    // all pending.
    let reasons = pending(&pi.root, &host);
    assert_eq!(reasons.len(), 5, "{reasons:?}");
    assert!(
        reasons
            .iter()
            .any(|r| r.starts_with("hdmi_enable_4kp60=1 not active"))
    );
    assert!(reasons.iter().any(|r| r.starts_with("fan control appears")));
    // After the reboot.
    host.vcgencmd = "core_freq_min=500\nhdmi_enable_4kp60=1".into();
    write("proc/device-tree/gpio-fan@0/compatible", b"gpio-fan\0");
    write(&format!("{DECODER_NODE}/memory-region"), &[0, 0, 0, 1]);
    write(
        "sys/module/rpi_hevc_dec/srcversion",
        format!("{}\n", embedded_srcversion().unwrap()).as_bytes(),
    );
    assert_eq!(pending(&pi.root, &host), Vec::<String>::new());
    // A smaller pool than configured (an old install before its reboot).
    write(
        "proc/device-tree/reserved-memory/linux,cma/size",
        &[0x20, 0, 0, 0],
    );
    assert_eq!(pending(&pi.root, &host).len(), 1);
    // Not a Pi at all: nothing to report.
    assert!(pending(&pi.root.join("nowhere"), &host).is_empty());
}

#[test]
fn remove_and_enable_are_reversible() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let host = Fake::new(&pi.root);
    let o = pi.options(KERNEL_RELEASE);
    apply(&o, &host);
    let mut state = pi.state();
    state.module_disabled = Some(Disabled {
        module_sha256: module_sha256(),
        reason: "removed by hand".into(),
    });
    state.save(&o).unwrap();
    apply(&o, &host);
    assert!(!pi.module().exists() && !pi.overlay().exists());
    assert!(host.held.borrow().is_empty());
    assert_eq!(
        pi.config(),
        format!(
            "{IMAGE_CONFIG}\n{}",
            BLOCK.replace("dtoverlay=ungoggled-hevc-cma\n", "")
        )
    );
    let mut state = pi.state();
    state.module_disabled = None;
    state.save(&o).unwrap();
    apply(&o, &host);
    assert_eq!(pi.config(), format!("{IMAGE_CONFIG}\n{BLOCK}"));
    assert!(pi.module().is_file());
}

#[test]
fn someone_elses_4k60_and_fan_settings_are_respected() {
    let base = "dtoverlay=vc4-kms-v3d,cma-size=0x28000000\n";
    // Already set by hand: not repeated, nothing missing.
    let e = edit_config(
        format!("{base}[all]\nhdmi_enable_4kp60=1\ndtoverlay=gpio-fan,gpiopin=12,temp=70000\n")
            .as_bytes(),
        false,
        false,
    )
    .unwrap();
    assert_eq!(e.text.matches("hdmi_enable_4kp60").count(), 1);
    assert_eq!(e.text.matches("gpio-fan").count(), 1, "{}", e.text);
    assert!(e.text.contains("gpiopin=12,temp=70000"));
    assert!(e.missing.is_empty(), "{:?}", e.missing);
    // Switched off on purpose: left alone and reported.
    let e = edit_config(
        format!("{base}hdmi_enable_4kp60=0\n").as_bytes(),
        false,
        false,
    )
    .unwrap();
    assert_eq!(e.text.matches("hdmi_enable_4kp60").count(), 1);
    assert_eq!(e.missing, ["hdmi_enable_4kp60"]);
    // Under a filter that does not match a Pi 4 it does not count.
    let e = edit_config(
        format!("{base}[pi5]\nhdmi_enable_4kp60=0\ndtoverlay=gpio-fan,gpiopin=2\n").as_bytes(),
        false,
        false,
    )
    .unwrap();
    assert!(e.text.contains("hdmi_enable_4kp60=1"));
    assert!(e.text.contains("dtoverlay=gpio-fan,gpiopin=14,temp=60000"));
    // A different overlay whose name merely starts like the fan's is not one.
    let e = edit_config(
        format!("{base}dtoverlay=gpio-fan-extra\n").as_bytes(),
        false,
        false,
    )
    .unwrap();
    assert!(e.text.contains("dtoverlay=gpio-fan,gpiopin=14"));
}

/// `hdmi_enable_4kp60` together with an overclocked HEVC block or a changed
/// voltage killed the firmware on the test Pi: such a config.txt must not
/// get the line, whatever else is in it.
#[test]
fn hdmi_4kp60_is_never_added_next_to_hevc_overclock_or_voltage_changes() {
    let base = "dtoverlay=vc4-kms-v3d,cma-size=0x28000000\n";
    for (tuning, line) in [
        ("hevc_freq=600\n", "hevc_freq=600"),
        ("hevc_freq=700\n", "hevc_freq=700"),
        ("hevc_freq=fast\n", "hevc_freq=fast"),
        ("over_voltage_delta=50000\n", "over_voltage_delta=50000"),
        ("over_voltage=2\n", "over_voltage=2"),
        ("[all]\nhevc_freq=650\n", "hevc_freq=650"),
    ] {
        let e = edit_config(format!("{base}{tuning}").as_bytes(), false, false).unwrap();
        assert!(
            !e.text.contains("hdmi_enable_4kp60"),
            "{line} must keep 4kp60 out:\n{}",
            e.text
        );
        assert_eq!(e.missing, ["hdmi_enable_4kp60"], "{line}");
        assert!(e.text.contains(line), "the owner's tuning stays");
        // The fan and the rest of the block are unaffected.
        assert!(e.text.contains("dtoverlay=gpio-fan,gpiopin=14,temp=60000"));
    }
    // Harmless values do not stop it, and neither does a filter for another Pi.
    for fine in [
        "hevc_freq=500\n",
        "hevc_freq=550\n",
        "over_voltage=0\n",
        "over_voltage_delta=0\n",
        "[pi5]\nhevc_freq=900\n",
    ] {
        let e = edit_config(format!("{base}{fine}").as_bytes(), false, false).unwrap();
        assert!(
            e.text.contains("hdmi_enable_4kp60=1"),
            "{fine}:\n{}",
            e.text
        );
        assert!(e.missing.is_empty(), "{fine}: {:?}", e.missing);
    }
    // A line from an earlier release is taken out again once such tuning appears.
    let before = edit_config(base.as_bytes(), false, false).unwrap();
    assert!(before.text.contains("hdmi_enable_4kp60=1"));
    let tuned = format!("{}hevc_freq=600\n", before.text);
    let after = edit_config(tuned.as_bytes(), false, false).unwrap();
    assert!(!after.text.contains("hdmi_enable_4kp60"), "{}", after.text);
}

/// The test Pi's `config.txt` after the owner had added both lines by hand
/// next to the block: a run keeps them and only closes the overlay's
/// parameter scope at the end of the block; the next run changes nothing.
#[test]
fn lines_added_by_hand_next_to_the_block_survive_a_run() {
    let config = |closer: &str| {
        format!(
            "{IMAGE_CONFIG}\n# >>> ungoggled system fixes (managed automatically, edits here are replaced)\n\
             [all]\ncore_freq_min=500\ndtoverlay=ungoggled-hevc-cma\n{closer}# <<< ungoggled system fixes\n\
             hdmi_enable_4kp60=1\n[all]\ndtoverlay=gpio-fan,gpiopin=14,temp=60000\n"
        )
    };
    let e = edit_config(config("").as_bytes(), true, false).unwrap();
    assert_eq!(e.text, config("dtoverlay=\n"));
    assert!(e.missing.is_empty(), "{:?}", e.missing);
    assert_eq!(
        edit_config(e.text.as_bytes(), true, false).unwrap().text,
        e.text
    );
}

/// S2: a `dtparam=` line appended below the block must not end up as a
/// parameter of our last overlay.
#[test]
fn the_block_closes_its_overlay_parameter_scope() {
    let e = edit_config(b"dtoverlay=vc4-kms-v3d,cma-size=0x28000000\n", true, false).unwrap();
    assert!(
        e.text
            .ends_with("dtoverlay=ungoggled-hevc-cma\ndtoverlay=\n# <<< ungoggled system fixes\n"),
        "{}",
        e.text
    );
    // Lines appended below by someone else stay where they are.
    let appended = format!("{}dtparam=i2c_arm=on\n", e.text);
    let again = edit_config(appended.as_bytes(), true, false).unwrap();
    assert_eq!(again.text, appended);
    // The empty overlay line is ours, not a foreign overlay, and pending()
    // does not trip over it.
    assert!(again.missing.is_empty(), "{:?}", again.missing);
    // A block without any overlay line gets no closer.
    let e = edit_config(
        format!(
            "dtoverlay=vc4-kms-v3d,cma-size=0x28000000\ndtoverlay=gpio-fan,gpiopin=12\n{NO_FAN}\n"
        )
        .as_bytes(),
        false,
        false,
    )
    .unwrap();
    assert!(!e.text.contains("dtoverlay=\n"), "{}", e.text);
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// S3: GPIO 14 used by the app's settings, by the serial console, or opted
/// out of by the owner: no fan line from us, everything else unchanged.
#[test]
fn the_fan_line_stays_out_when_gpio_14_has_another_user() {
    let base = "dtoverlay=vc4-kms-v3d,cma-size=0x28000000\n";
    let fan = "dtoverlay=gpio-fan,gpiopin=14,temp=60000";
    let e = edit_config(base.as_bytes(), false, true).unwrap();
    assert!(!e.text.contains("gpio-fan"), "{}", e.text);
    assert!(e.text.contains("core_freq_min=500"));
    assert!(e.missing.is_empty(), "{:?}", e.missing);
    assert!(
        e.notes
            .iter()
            .any(|n| n == "gpio-fan: skipped: GPIO 14 is used by the settings"),
        "{:?}",
        e.notes
    );
    for (extra, note) in [
        (format!("{NO_FAN}\n"), "is in config.txt"),
        ("#UNGOGGLED:   no-fan\n".to_owned(), "is in config.txt"),
        (
            "[pi5]\n# ungoggled: no-fan\n".to_owned(),
            "is in config.txt",
        ),
        ("enable_uart=1\n".to_owned(), "enable_uart=1"),
        ("[all]\nenable_uart=1\n".to_owned(), "enable_uart=1"),
    ] {
        let e = edit_config(format!("{base}{extra}").as_bytes(), false, false).unwrap();
        assert!(!e.text.contains(fan), "{extra}:\n{}", e.text);
        assert!(e.text.contains(&extra), "{extra} stays");
        assert!(e.notes.iter().any(|n| n.contains(note)), "{:?}", e.notes);
        assert!(e.missing.is_empty(), "{extra}: {:?}", e.missing);
    }
    // Not an opt-out: switched off UART, a UART for another Pi, a marker
    // inside the block (replaced on every run), other comments.
    for extra in [
        "enable_uart=0\n",
        "[pi5]\nenable_uart=1\n",
        "# ungoggled: no-fan please\n",
        "#dtoverlay=gpio-fan\n",
    ] {
        let e = edit_config(format!("{base}{extra}").as_bytes(), false, false).unwrap();
        assert!(e.text.contains(fan), "{extra}:\n{}", e.text);
    }
    let with_block = edit_config(base.as_bytes(), false, false).unwrap().text;
    let marker_inside = with_block.replace("[all]\n", &format!("[all]\n{NO_FAN}\n"));
    let e = edit_config(marker_inside.as_bytes(), false, false).unwrap();
    assert_eq!(e.text, with_block);
    // A fan line from an earlier run goes once the opt-out appears.
    let e = edit_config(format!("{with_block}{NO_FAN}\n").as_bytes(), false, false).unwrap();
    assert!(!e.text.contains(fan), "{}", e.text);
}

#[test]
fn the_settings_pins_decide_about_the_fan_line() {
    // No default pin of the app is the fan's.
    for pin in [
        crate::hardware::DEFAULT_BEEPER_PIN,
        crate::hardware::DEFAULT_POWER_BUTTON_PIN,
        crate::hardware::DEFAULT_GOGGLES_LED_PIN,
        crate::hardware::DEFAULT_VIDEO_LED_PIN,
        crate::hardware::DEFAULT_HDMI_LED_PIN,
    ] {
        assert_ne!(u32::from(pin), FAN_GPIO);
    }
    let fan = "dtoverlay=gpio-fan,gpiopin=14,temp=60000";
    for (settings, listed) in [
        (None, true),
        (
            Some(&br#"{"beeper_pin":6,"buttons":[{"pin":5,"action":"reboot"}]}"#[..]),
            true,
        ),
        (Some(&br#"{"beeper_pin":14}"#[..]), false),
        (
            Some(&br#"{"hdmi_led_pin":14,"beeper_pin":null}"#[..]),
            false,
        ),
        (
            Some(
                &br#"{"buttons":[{"pin":9,"action":"reboot"},{"pin":14,"action":"shutdown"}]}"#[..],
            ),
            false,
        ),
        // A field this release does not know yet still counts.
        (Some(&br#"{"fan_relay_pin":14}"#[..]), false),
        // Damaged or not an object: no pins.
        (Some(&b"{\"beeper_pin\":14"[..]), true),
        (Some(&b"[14]"[..]), true),
        (Some(&b""[..]), true),
    ] {
        let pi = Pi::new(Some(IMAGE_CONFIG));
        if let Some(s) = settings {
            write(&pi.root, SETTINGS, s);
        }
        let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
        assert_eq!(
            pi.config().contains(fan),
            listed,
            "{:?}",
            settings.map(String::from_utf8_lossy)
        );
        assert!(r.missing.is_empty(), "{r:?}");
    }
    // The pin given back in the settings: the line returns on the next run.
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let host = Fake::new(&pi.root);
    write(&pi.root, SETTINGS, br#"{"beeper_pin":14}"#);
    apply(&pi.options(KERNEL_RELEASE), &host);
    assert!(!pi.config().contains(fan));
    write(&pi.root, SETTINGS, br#"{"beeper_pin":6}"#);
    apply(&pi.options(KERNEL_RELEASE), &host);
    assert_eq!(pi.config(), format!("{IMAGE_CONFIG}\n{BLOCK}"));
}

/// S4: the packages put on hold are on disk before the module check, which
/// may sit in modprobe for up to 30 s; a run killed there keeps the record.
#[test]
fn the_hold_is_recorded_before_the_module_check() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    fs::create_dir_all(pi.root.join(DECODER_NODE)).unwrap();
    let host = Fake::new(&pi.root);
    let mut o = pi.options(KERNEL_RELEASE);
    o.live = true;
    apply(&o, &host);
    assert_eq!(host.calls("modprobe").len(), 1);
    let saved: State =
        serde_json::from_str(host.state_at_modprobe.borrow().as_deref().unwrap()).unwrap();
    assert_eq!(saved.held.len(), 7, "{saved:?}");
    assert_eq!(saved.held, *host.held.borrow());
}

/// S5: an install that keeps failing is tried once per boot, not before
/// every service start, and writes nothing to the boot partition.
#[test]
fn a_failing_install_is_retried_once_per_boot() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let mut host = Fake::new(&pi.root);
    host.chosen = Some("/lib/modules/x/extra/rpi-hevc-dec.ko\n".into());
    // The decoder is loaded (stock), so check_loaded never probes.
    fs::create_dir_all(pi.root.join("sys/module/rpi_hevc_dec")).unwrap();
    write(&pi.root, BOOT_ID, b"boot-1\n");
    let mut o = pi.options(KERNEL_RELEASE);
    o.live = true;
    let r = apply(&o, &host);
    assert!(r.lines[0].contains("not installed"), "{r:?}");
    assert!(!pi.module().exists() && !pi.overlay().exists());
    assert!(!pi.config().contains("ungoggled-hevc-cma"));
    assert!(
        pi.state()
            .install_failed
            .is_some_and(|f| f.boot_id == "boot-1" && f.reason.contains("instead"))
    );
    // Same boot: no install, no depmod, no overlay file.
    host.clear();
    let r = apply(&o, &host);
    assert!(r.lines[0].contains("failed earlier in this boot"), "{r:?}");
    assert!(host.calls("depmod").is_empty() && host.calls("modinfo").len() == 1);
    assert!(!pi.module().exists() && !pi.overlay().exists());
    assert!(r.missing.contains(&"module"));
    // After a reboot it is tried again, and a success clears the record.
    write(&pi.root, BOOT_ID, b"boot-2\n");
    host.chosen = None;
    host.clear();
    let r = apply(&o, &host);
    assert!(
        r.lines[0].starts_with("HEVC decoder module: installed"),
        "{r:?}"
    );
    assert!(pi.module().is_file() && pi.overlay().is_file());
    assert!(pi.config().contains("ungoggled-hevc-cma"));
    assert_eq!(pi.state().install_failed, None);
    // Offline (an image build) there is no boot id: always tried, never
    // recorded.
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let mut host = Fake::new(&pi.root);
    host.chosen = Some("/lib/modules/x/extra/rpi-hevc-dec.ko\n".into());
    apply(&pi.options(KERNEL_RELEASE), &host);
    assert_eq!(pi.state().install_failed, None);
    host.clear();
    apply(&pi.options(KERNEL_RELEASE), &host);
    assert_eq!(host.calls("depmod").len(), 2);
}

/// S6: a `config.txt` that is gone, empty or full of NUL bytes is restored
/// from the backup before the latest change, then fixed as usual.
#[test]
fn a_lost_config_is_restored_from_the_previous_backup() {
    let damaged: &[u8] = b"dtoverlay=vc4\0\0\0\0\0\0";
    for lost in [None, Some(&b""[..]), Some(damaged)] {
        let pi = Pi::new(Some(IMAGE_CONFIG));
        let host = Fake::new(&pi.root);
        apply(&pi.options(KERNEL_RELEASE), &host);
        let good = pi.config();
        let path = pi.root.join(BOOT).join("config.txt");
        match lost {
            None => fs::remove_file(&path).unwrap(),
            Some(b) => fs::write(&path, b).unwrap(),
        }
        let r = apply(&pi.options(KERNEL_RELEASE), &host);
        assert!(r.lines.iter().any(|l| l.contains("restored")), "{r:?}");
        assert!(r.missing.is_empty(), "{r:?}");
        assert_eq!(pi.config(), good, "{lost:?}");
        let kept = pi.root.join(BOOT).join("config.txt.ungoggled-damaged");
        assert_eq!(
            fs::read(&kept).ok().as_deref(),
            lost.filter(|b| !b.is_empty()),
            "{lost:?}"
        );
        assert!(pi.leftovers().is_empty());
    }
    // Shapes that may be the owner's doing are not overwritten.
    for odd in [
        format!("{BLOCK}{BLOCK}").into_bytes(),
        "x".repeat(MAX_CONFIG + 1).into_bytes(),
        b"a=1\n\xff\n".to_vec(),
    ] {
        let pi = Pi::new(Some(IMAGE_CONFIG));
        apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
        fs::write(pi.root.join(BOOT).join("config.txt"), &odd).unwrap();
        let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
        assert_eq!(
            fs::read(pi.root.join(BOOT).join("config.txt")).unwrap(),
            odd
        );
        assert!(r.missing.contains(&"config"));
    }
    // A backup this code would not edit is not used either.
    let pi = Pi::new(None);
    let prev = pi.root.join(BOOT).join("config.txt.ungoggled-prev");
    fs::write(&prev, format!("{BLOCK}{BLOCK}")).unwrap();
    let r = apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
    assert!(!pi.root.join(BOOT).join("config.txt").exists());
    assert!(
        r.lines.iter().any(|l| l.contains("cannot restore")),
        "{r:?}"
    );
    assert!(r.missing.contains(&"config"));
}

/// S7: values the firmware prints in another form, and the overlay file in
/// the branch where the kernel package cannot be checked.
#[test]
fn pending_compares_numbers_and_needs_the_overlay_file() {
    assert!(same_value("0500", "500"));
    assert!(same_value("0x1f4", "500"));
    assert!(same_value(" 1", "1"));
    assert!(!same_value("500", "550"));
    assert!(!same_value("fast", "500"));
    assert!(same_value("fast", "fast"));
    let pi = Pi::new(Some(
        "dtoverlay=vc4-kms-v3d,cma-size=0x28000000\ncore_freq_min=0500\nhdmi_enable_4kp60=01\n",
    ));
    let mut host = Fake::new(&pi.root);
    host.vcgencmd = "core_freq_min=500\nhdmi_enable_4kp60=1".into();
    assert_eq!(pending(&pi.root, &host), Vec::<String>::new());
    host.vcgencmd = "core_freq_min=250\nhdmi_enable_4kp60=1".into();
    assert_eq!(pending(&pi.root, &host).len(), 1);

    // Module in place, overlay file lost, dpkg-query not answering: the
    // overlay line goes instead of promising a pool that never appears.
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let mut host = Fake::new(&pi.root);
    apply(&pi.options(KERNEL_RELEASE), &host);
    assert!(pi.config().contains("dtoverlay=ungoggled-hevc-cma"));
    fs::remove_file(pi.overlay()).unwrap();
    host.dpkg_ok = false;
    apply(&pi.options(KERNEL_RELEASE), &host);
    assert!(pi.module().is_file());
    assert!(
        !pi.config().contains("ungoggled-hevc-cma"),
        "{}",
        pi.config()
    );
    // Back once the check runs again.
    host.dpkg_ok = true;
    apply(&pi.options(KERNEL_RELEASE), &host);
    assert!(pi.overlay().is_file() && pi.config().contains("ungoggled-hevc-cma"));
}

#[test]
fn pending_inputs_change_with_what_the_cli_changes() {
    let pi = Pi::new(Some(IMAGE_CONFIG));
    let before = pending_inputs(&pi.root);
    apply(&pi.options(KERNEL_RELEASE), &Fake::new(&pi.root));
    let after = pending_inputs(&pi.root);
    assert_ne!(before, after);
    assert!(after[..3].iter().all(Option::is_some), "{after:?}");
    assert_eq!(pending_inputs(&pi.root), after);
}
