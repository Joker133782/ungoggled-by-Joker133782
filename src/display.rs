//! Set the HDMI scanout mode independently of the changing camera resolution.
use anyhow::{Context, Result, bail};
use drm::{
    Device as _,
    buffer::DrmFourcc,
    control::{Device as _, Mode, ModeFlags, connector, framebuffer},
};
use std::{
    fs::{self, File, OpenOptions},
    os::fd::{AsFd, BorrowedFd},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Card(File);
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl drm::Device for Card {}
impl drm::control::Device for Card {}

pub struct Display {
    _card: Arc<Card>,
    pub info: serde_json::Value,
    /// The mode on screen, in mpv's `--drm-mode` syntax (see `mpv_mode`).
    pub mpv_mode: String,
    upgrade: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    // The DRM file owns the black primary framebuffer and its backing buffer.
    // Keep it open across decoder restarts; kmssink owns only the video overlay.
}

fn interlaced(m: &Mode) -> bool {
    m.flags().contains(ModeFlags::INTERLACE)
}

/// `1920x1080@60` for progressive and `1920x1080@60i` for interlaced modes,
/// whose refresh rate is the field rate.
fn mode_id(m: &Mode) -> String {
    let scan = if interlaced(m) { "i" } else { "" };
    format!("{}x{}@{}{scan}", m.size().0, m.size().1, m.vrefresh())
}

/// `m` as mpv's `--drm-mode=WxH@R`, so USB playback keeps the mode already
/// on screen instead of switching to the monitor's preferred one (e.g. 4K
/// at 30 Hz). mpv matches R to within 0.01 Hz of its own refresh
/// calculation, repeated here, and a spec matching no mode fails mpv's
/// video output entirely, so the rounded `vrefresh` (60 for 59.94) won't do.
fn mpv_mode(m: &Mode) -> String {
    let (_, _, htotal) = m.hsync();
    let (_, _, vtotal) = m.vsync();
    let mut hz = f64::from(m.clock()) * 1000.0 / (f64::from(htotal) * f64::from(vtotal));
    if interlaced(m) {
        hz *= 2.0;
    }
    format!("{}x{}@{hz:.2}", m.size().0, m.size().1)
}

fn select_mode(modes: &[Mode], requested: &str) -> Option<Mode> {
    if requested != "auto"
        && let Some(m) = modes.iter().find(|m| mode_id(m) == requested)
    {
        return Some(*m);
    }
    // Automatic selection never picks an interlaced mode.
    let progressive = |m: &&Mode| !interlaced(m);
    modes
        .iter()
        .filter(progressive)
        .filter(|m| m.size() == (1920, 1080) && m.vrefresh() <= 60)
        .max_by_key(|m| m.vrefresh())
        .copied()
        .or_else(|| {
            modes
                .iter()
                .filter(progressive)
                .filter(|m| m.size().0 <= 1920 && m.size().1 <= 1080)
                .max_by_key(|m| (u32::from(m.size().0) * u32::from(m.size().1), m.vrefresh()))
                .copied()
        })
}

impl Display {
    pub fn prepare(
        requested: Option<u32>,
        mode_request: &str,
        fallback: Option<&Path>,
    ) -> Result<Self> {
        let mut paths: Vec<_> = fs::read_dir("/dev/dri")?
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("card"))
            .map(|e| e.path())
            .collect();
        paths.sort();
        // Only meaningful when nothing matches below: tells a connected cable
        // on the *other* micro-HDMI port apart from no cable anywhere, since
        // the fix ("move it to the port next to the USB-C power input") is
        // different from "plug in a cable".
        let mut other_connector_live = false;
        for path in paths {
            let card = Card(OpenOptions::new().read(true).write(true).open(path)?);
            let Ok(resources) = card.resource_handles() else {
                continue;
            };
            for &handle in resources.connectors() {
                let con = card.get_connector(handle, true)?;
                if con.interface() != connector::Interface::HDMIA {
                    continue;
                }
                // No connector chosen in settings: use whichever micro-HDMI
                // port actually has a display, instead of always requiring
                // the first one specifically.
                if requested.is_some_and(|id| id != u32::from(handle)) {
                    if con.state() == connector::State::Connected {
                        other_connector_live = true;
                    }
                    continue;
                }
                if con.state() != connector::State::Connected {
                    continue;
                }
                let mode = select_mode(con.modes(), mode_request)
                    .context("No supported progressive HDMI mode")?;
                let encoder = con
                    .current_encoder()
                    .or_else(|| con.encoders().first().copied())
                    .context("HDMI encoder missing")?;
                let encoder = card.get_encoder(encoder)?;
                let crtc = encoder
                    .crtc()
                    .or_else(|| {
                        resources
                            .filter_crtcs(encoder.possible_crtcs())
                            .first()
                            .copied()
                    })
                    .context("HDMI CRTC missing")?;
                // Always own a primary framebuffer, including when the mode is already
                // correct. The live decoder uses an overlay; its removal reveals this image.
                card.acquire_master_lock()
                    .context("Acquire DRM for HDMI image")?;
                let result = (|| -> Result<framebuffer::Handle> {
                    let (w, h) = mode.size();
                    let mut buffer =
                        card.create_dumb_buffer((w.into(), h.into()), DrmFourcc::Xrgb8888, 32)?;
                    use drm::buffer::Buffer as _;
                    let pitch = buffer.pitch() as usize;
                    let pixels = crate::assets::pixels(fallback, w.into(), h.into());
                    let mut mapped = card.map_dumb_buffer(&mut buffer)?;
                    for (y, row) in pixels.chunks_exact(w as usize * 4).enumerate() {
                        mapped.as_mut()[y * pitch..y * pitch + row.len()].copy_from_slice(row);
                    }
                    drop(mapped);
                    let fb = card.add_framebuffer(&buffer, 24, 32)?;
                    card.set_crtc(crtc, Some(fb), (0, 0), &[handle], Some(mode))?;
                    Ok(fb)
                })();
                let release = card.release_master_lock();
                let fb = result.context("Set HDMI mode and fallback")?;
                release?;
                let modes: Vec<_> = con.modes().iter().map(mode_id).collect();
                let info = serde_json::json!({"hdmi_width":mode.size().0,"hdmi_height":mode.size().1,"hdmi_hz":mode.vrefresh(),"hdmi_interlaced":interlaced(&mode),"hdmi_modes":modes,"hdmi_requested_mode":mode_request});
                let card = Arc::new(card);
                let upgrade = Arc::new(AtomicBool::new(false));
                let stop = Arc::new(AtomicBool::new(false));
                {
                    let preferred = mode_request.to_owned();
                    // EDID probing may block for a second on this legacy kernel.
                    // Keep it off the decoder thread and reuse the same DRM fd.
                    let probe = card.clone();
                    let pending = upgrade.clone();
                    let stopped = stop.clone();
                    // With no port chosen in settings, losing the display here
                    // may mean it moved to the *other* HDMI-A port: trigger a
                    // full re-scan instead of staying locked onto this now-dark
                    // one forever. A manually selected port, on the other hand,
                    // should keep reporting its own connection state as-is.
                    let auto = requested.is_none();
                    thread::spawn(move || {
                        while !stopped.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_secs(3));
                            if stopped.load(Ordering::Relaxed) {
                                break;
                            }
                            let Ok(con) = probe.get_connector(handle, true) else {
                                continue;
                            };
                            let upgraded = select_mode(con.modes(), &preferred).is_some_and(|m| {
                                m != mode && (preferred != "auto" || m.size().0 > mode.size().0)
                            });
                            let moved = auto && con.state() != connector::State::Connected;
                            // Someone else put a framebuffer on the CRTC: the
                            // kernel console after the monitor was switched off
                            // and on (a hotplug gives it the output) or after a
                            // VT switch. Nothing of ours is on screen then, and
                            // nothing else would ever take the output back, so
                            // prepare the display again (a monitor that is
                            // still away makes that fail and be retried).
                            let taken = probe
                                .get_crtc(crtc)
                                .is_ok_and(|c| c.framebuffer() != Some(fb));
                            if upgraded || moved || taken {
                                pending.store(true, Ordering::Relaxed);
                                break;
                            }
                        }
                    });
                }
                return Ok(Self {
                    _card: card,
                    info,
                    mpv_mode: mpv_mode(&mode),
                    upgrade,
                    stop,
                });
            }
        }
        if requested.is_some() && other_connector_live {
            bail!(
                "A display is connected, but not to the HDMI port selected in \
                 Settings. Pick the other port there, or leave it on \"Automatic\"."
            )
        } else if requested.is_some() {
            bail!("No display connected on the HDMI port selected in Settings")
        } else {
            bail!("No display connected on either HDMI port")
        }
    }

    pub fn needs_upgrade(&self) -> bool {
        // A headless boot may expose only VGA. Upgrade when EDID arrives, but
        // keep full HD scanout stable when the monitor is temporarily unplugged.
        // Also true when the picture is no longer on the output (see `taken`).
        self.upgrade.load(Ordering::Relaxed)
    }
}
impl Drop for Display {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
