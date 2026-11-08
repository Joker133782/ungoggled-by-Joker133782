use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub hdmi_mode: String,
    /// DRM connector id to render to (as listed by `/api/hdmi-connectors`),
    /// e.g. when two displays are plugged in at once and only one should
    /// show the picture. `None` uses whichever HDMI-A port has a display
    /// connected, trying each in a stable order.
    pub hdmi_connector: Option<u32>,
    pub fallback_image: Option<String>,
    pub preview_enabled: bool,
    /// BCM GPIO pin driving a piezo beeper; defaults to GPIO6.
    pub beeper_pin: Option<u8>,
    /// BCM GPIO pin for a shutdown button; defaults to the NuclearHazard hat's
    /// power button (GPIO19, active low).
    pub power_button_pin: Option<u8>,
    /// BCM GPIO pin driving a plain LED that lights while the goggles are attached.
    pub goggles_led_pin: Option<u8>,
    /// BCM GPIO pin driving a plain LED that lights while video is live.
    pub video_led_pin: Option<u8>,
    /// BCM GPIO pin driving a plain LED that lights while an HDMI display is connected.
    pub hdmi_led_pin: Option<u8>,
    /// Beep every few seconds until the clock has been synced once since boot.
    pub beep_until_time_synced: bool,
    /// Relay the live feed over the Wi-Fi access point for spectators, at
    /// `stream_quality`. Off by default: this is a deliberate, operator-set
    /// choice, not something that starts on its own.
    pub stream_enabled: bool,
    /// `"high"`: relay the original goggles stream unmodified, for one or
    /// two viewers. `"event"`: transcode to a lower bitrate so the Pi's own
    /// Wi-Fi can carry more viewers at once, at reduced quality.
    pub stream_quality: String,
    /// Buffer every live session (in RAM, under the runtime tmpfs) and save
    /// it to the SD card as soon as the feed drops, keeping the 5 most
    /// recent; no manual start/stop needed. The manual recording button
    /// still works independently of this.
    pub auto_record: bool,
    /// Like the goggles' own instant-replay: the ~30 s ring buffer always
    /// runs in the background regardless of this setting; this only decides
    /// whether a feed drop (e.g. a crash) auto-saves it, keeping the most
    /// recent 3 clips. The "save last 30 seconds" button still works
    /// independently of this.
    pub crash_buffer: bool,
    /// Write recordings to the first detected USB mass-storage drive instead
    /// of the SD card; falls back to the SD card whenever none is mounted.
    pub record_to_usb: bool,
    /// When a Wi-Fi client join (`wifi::join`) is active, keep it from
    /// autoconnecting on a future boot — the Pi always comes back up
    /// broadcasting its own access point until someone explicitly joins a
    /// network again. Off lets it resume the last-joined network instead,
    /// for a Pi that stays put as a stationary review station.
    pub wifi_always_ap: bool,
    /// Whether the browser terminal (a root shell, `terminal.rs`) can be
    /// opened at all. Off by default; even when on, every opening needs the
    /// password again. Turning it off closes an open terminal.
    pub terminal_enabled: bool,
    /// Stretch the live goggles picture over the whole screen instead of
    /// keeping its shape (a 4:3 picture on a 16:9 screen otherwise gets black
    /// bars at the sides). Off by default: the picture is shown as it arrives.
    pub stretch_live: bool,
    /// The same for a video played from a USB drive.
    pub stretch_usb: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            hdmi_mode: "auto".into(),
            hdmi_connector: None,
            fallback_image: Some("default-loop.h264".into()),
            preview_enabled: true,
            beeper_pin: Some(crate::hardware::DEFAULT_BEEPER_PIN),
            power_button_pin: Some(crate::hardware::DEFAULT_POWER_BUTTON_PIN),
            goggles_led_pin: Some(crate::hardware::DEFAULT_GOGGLES_LED_PIN),
            video_led_pin: Some(crate::hardware::DEFAULT_VIDEO_LED_PIN),
            hdmi_led_pin: Some(crate::hardware::DEFAULT_HDMI_LED_PIN),
            beep_until_time_synced: false,
            stream_enabled: false,
            stream_quality: "high".into(),
            auto_record: false,
            crash_buffer: false,
            record_to_usb: false,
            wifi_always_ap: true,
            terminal_enabled: false,
            stretch_live: false,
            stretch_usb: false,
        }
    }
}
#[derive(Clone)]
pub struct Store {
    pub dir: PathBuf,
    pub value: Arc<Mutex<Settings>>,
}
impl Store {
    pub fn open(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(dir.join("images/thumbs"))?;
        let first_run = !dir.join("settings.json").exists();
        // "No signal" is a permanent built-in like the videos below (see
        // `assets::BUILTINS`); older versions let it be deleted, so put it
        // back if it's missing rather than only on first run.
        let no_signal = dir.join("images/no-signal.png");
        if !no_signal.exists() {
            atomic_write(&no_signal, include_bytes!("../assets/no-signal.png"))?;
        }
        // The three built-in waiting-screen videos are refreshed on every
        // start (not just first run), so an in-place update replaces their
        // content for an already-provisioned Pi, not only a fresh flash.
        // They're treated as permanent menu entries, not user content. Their
        // list thumbnails ship as pre-rendered posters: frames from the raw
        // .h264 can't be grabbed with the mpv recipe used for other videos.
        for (id, bytes, poster) in [
            (
                "default-loop.h264",
                &include_bytes!("../assets/default-loop.h264")[..],
                &include_bytes!("../assets/default-loop.jpg")[..],
            ),
            (
                "default-loop-3d.h264",
                &include_bytes!("../assets/default-loop-3d.h264")[..],
                &include_bytes!("../assets/default-loop-3d.jpg")[..],
            ),
            (
                "default-comic.h264",
                &include_bytes!("../assets/default-comic.h264")[..],
                &include_bytes!("../assets/default-comic.jpg")[..],
            ),
        ] {
            atomic_write(&dir.join("images").join(id), bytes)?;
            atomic_write(
                &dir.join("images").join(id).with_extension("json"),
                &serde_json::to_vec(&crate::assets::VideoInfo {
                    width: 1920,
                    height: 1080,
                    fps: 30.0,
                })?,
            )?;
            atomic_write(&crate::assets::thumb_path(&dir.join("images"), id), poster)?;
        }
        let value = match fs::read(dir.join("settings.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(e.into()),
        };
        if first_run {
            atomic_write(
                &dir.join("settings.json"),
                &serde_json::to_vec_pretty(&value)?,
            )?;
        }
        Ok(Self {
            dir,
            value: Arc::new(Mutex::new(value)),
        })
    }
    pub fn get(&self) -> Settings {
        self.value.lock().unwrap().clone()
    }
    pub fn save(&self, value: Settings) -> Result<()> {
        if !valid_mode(&value.hdmi_mode) {
            bail!("Invalid HDMI mode");
        }
        if !valid_stream_quality(&value.stream_quality) {
            bail!("Invalid stream quality");
        }
        if let Some(id) = &value.fallback_image
            && (!valid_id(id) || !self.dir.join("images").join(id).is_file())
        {
            bail!("Unknown fallback image");
        }
        let pins = [
            value.beeper_pin,
            value.power_button_pin,
            value.goggles_led_pin,
            value.video_led_pin,
            value.hdmi_led_pin,
        ];
        if pins.into_iter().flatten().any(|p| !valid_pin(p)) {
            bail!("GPIO pins must be between 2 and 27");
        }
        let configured: Vec<u8> = pins.into_iter().flatten().collect();
        let distinct: std::collections::HashSet<u8> = configured.iter().copied().collect();
        if distinct.len() != configured.len() {
            bail!("GPIO pins must all be different");
        }
        // Serialize concurrent saves together with the file replacement.
        let mut current = self.value.lock().unwrap();
        atomic_write(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&value)?,
        )?;
        *current = value;
        Ok(())
    }
}
/// Fallback library entries: still images (`.png`) and looped videos (`.h264`).
pub fn valid_id(id: &str) -> bool {
    (id.ends_with(".png") || id.ends_with(".h264"))
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !id.contains("..")
}
pub fn valid_pin(pin: u8) -> bool {
    (2..=27).contains(&pin)
}
pub fn valid_stream_quality(quality: &str) -> bool {
    matches!(quality, "high" | "event")
}
pub fn valid_mode(mode: &str) -> bool {
    if mode == "auto" {
        return true;
    }
    let Some((size, hz)) = mode.split_once('@') else {
        return false;
    };
    let hz = hz.strip_suffix('i').unwrap_or(hz);
    let Some((w, h)) = size.split_once('x') else {
        return false;
    };
    match (w.parse::<u32>(), h.parse::<u32>(), hz.parse::<u32>()) {
        (Ok(w), Ok(h), Ok(hz)) => {
            (320..=7680).contains(&w) && (200..=4320).contains(&h) && (1..=240).contains(&hz)
        }
        _ => false,
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(tmp, path)?;
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}
fn pin(value: &serde_json::Value) -> Option<u8> {
    value
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
        .filter(|&p| valid_pin(p))
}
/// Stable updater entry point. Each release owns migration into its own schema.
/// Preserve recognized compatible values; report fields that need defaults.
pub fn migrate(data_dir: &Path, output: &Path) -> Result<Vec<String>> {
    let old: serde_json::Value = match fs::read(data_dir.join("settings.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.into()),
    };
    let object = old
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Settings file must contain an object"))?;
    let mut next = Settings::default();
    let mut warnings = Vec::new();
    for (key, value) in object {
        let accepted = match key.as_str() {
            "hdmi_mode" => value
                .as_str()
                .filter(|v| valid_mode(v))
                .map(|v| next.hdmi_mode = v.into())
                .is_some(),
            "hdmi_connector" if value.is_null() => {
                next.hdmi_connector = None;
                true
            }
            "hdmi_connector" => value
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .map(|v| next.hdmi_connector = Some(v))
                .is_some(),
            "preview_enabled" => value.as_bool().map(|v| next.preview_enabled = v).is_some(),
            "fallback_image" if value.is_null() => {
                next.fallback_image = None;
                true
            }
            "fallback_image" => value
                .as_str()
                .filter(|v| valid_id(v) && data_dir.join("images").join(v).is_file())
                .map(|v| next.fallback_image = Some(v.into()))
                .is_some(),
            "beeper_pin" if value.is_null() => {
                next.beeper_pin = None;
                true
            }
            "beeper_pin" => pin(value).map(|v| next.beeper_pin = Some(v)).is_some(),
            "power_button_pin" if value.is_null() => {
                next.power_button_pin = None;
                true
            }
            "power_button_pin" => pin(value)
                .map(|v| next.power_button_pin = Some(v))
                .is_some(),
            "goggles_led_pin" if value.is_null() => {
                next.goggles_led_pin = None;
                true
            }
            "goggles_led_pin" => pin(value).map(|v| next.goggles_led_pin = Some(v)).is_some(),
            "video_led_pin" if value.is_null() => {
                next.video_led_pin = None;
                true
            }
            "video_led_pin" => pin(value).map(|v| next.video_led_pin = Some(v)).is_some(),
            "hdmi_led_pin" if value.is_null() => {
                next.hdmi_led_pin = None;
                true
            }
            "hdmi_led_pin" => pin(value).map(|v| next.hdmi_led_pin = Some(v)).is_some(),
            "beep_until_time_synced" => value
                .as_bool()
                .map(|v| next.beep_until_time_synced = v)
                .is_some(),
            "stream_enabled" => value.as_bool().map(|v| next.stream_enabled = v).is_some(),
            "stream_quality" => value
                .as_str()
                .filter(|v| valid_stream_quality(v))
                .map(|v| next.stream_quality = v.into())
                .is_some(),
            "auto_record" => value.as_bool().map(|v| next.auto_record = v).is_some(),
            "crash_buffer" => value.as_bool().map(|v| next.crash_buffer = v).is_some(),
            "record_to_usb" => value.as_bool().map(|v| next.record_to_usb = v).is_some(),
            "wifi_always_ap" => value.as_bool().map(|v| next.wifi_always_ap = v).is_some(),
            "terminal_enabled" => value.as_bool().map(|v| next.terminal_enabled = v).is_some(),
            "stretch_live" => value.as_bool().map(|v| next.stretch_live = v).is_some(),
            "stretch_usb" => value.as_bool().map(|v| next.stretch_usb = v).is_some(),
            _ => false,
        };
        if !accepted {
            warnings.push(format!(
                "{key}: not compatible with this release; default used"
            ));
        }
    }
    if next
        .fallback_image
        .as_ref()
        .is_some_and(|id| !data_dir.join("images").join(id).is_file())
    {
        next.fallback_image = None;
    }
    // A migrated file could combine old values into a pin clash the current
    // schema forbids; drop to defaults rather than reject the whole migration.
    let pins = [
        next.beeper_pin,
        next.power_button_pin,
        next.goggles_led_pin,
        next.video_led_pin,
        next.hdmi_led_pin,
    ];
    let configured: Vec<u8> = pins.into_iter().flatten().collect();
    let distinct: std::collections::HashSet<u8> = configured.iter().copied().collect();
    if distinct.len() != configured.len() {
        next.beeper_pin = Settings::default().beeper_pin;
        next.power_button_pin = Settings::default().power_button_pin;
        next.goggles_led_pin = Settings::default().goggles_led_pin;
        next.video_led_pin = Settings::default().video_led_pin;
        next.hdmi_led_pin = Settings::default().hdmi_led_pin;
        warnings.push("conflicting GPIO pins; defaults used".into());
    }
    atomic_write(output, &serde_json::to_vec_pretty(&next)?)?;
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_and_invalid_modes() {
        for s in ["../a.png", "/a.png", "a/b.png", "a.png\n", "a..png"] {
            assert!(!valid_id(s));
        }
        assert!(valid_id("fallback-123.png"));
        for s in [
            "1920x1080@60",
            "1920x1080@60i",
            "3840x2160@30",
            "720x576@50i",
        ] {
            assert!(valid_mode(s));
        }
        for s in [
            "1920x1080@0",
            "1920x1080@600",
            "1920x1080@60p",
            "1920x1080@i",
            "auto\n",
        ] {
            assert!(!valid_mode(s));
        }
    }
    #[test]
    fn default_gpio_pins() {
        let s = Settings::default();
        assert_eq!(s.beeper_pin, Some(crate::hardware::DEFAULT_BEEPER_PIN));
        assert_eq!(
            s.power_button_pin,
            Some(crate::hardware::DEFAULT_POWER_BUTTON_PIN)
        );
        assert_eq!(
            s.goggles_led_pin,
            Some(crate::hardware::DEFAULT_GOGGLES_LED_PIN)
        );
        assert_eq!(
            s.video_led_pin,
            Some(crate::hardware::DEFAULT_VIDEO_LED_PIN)
        );
        assert_eq!(s.hdmi_led_pin, Some(crate::hardware::DEFAULT_HDMI_LED_PIN));
    }
    #[test]
    fn rejects_out_of_range_and_conflicting_pins() {
        let f = crate::update::tests::Fixture::new();
        let store = Store::open(f.0.clone()).unwrap();
        let mut s = store.get();
        s.beeper_pin = Some(1);
        assert!(store.save(s.clone()).is_err());
        s.beeper_pin = Some(21);
        s.power_button_pin = Some(21);
        assert!(store.save(s.clone()).is_err());
        s.power_button_pin = Some(19);
        store.save(s).unwrap();
        assert_eq!(store.get().beeper_pin, Some(21));
    }
    #[test]
    fn led_pins_reject_conflicts_across_all_gpio_features() {
        let f = crate::update::tests::Fixture::new();
        let store = Store::open(f.0.clone()).unwrap();
        let mut s = store.get();
        s.goggles_led_pin = Some(20);
        s.video_led_pin = Some(21);
        s.hdmi_led_pin = Some(20);
        assert!(store.save(s.clone()).is_err());
        s.hdmi_led_pin = Some(22);
        store.save(s).unwrap();
        let saved = store.get();
        assert_eq!(saved.goggles_led_pin, Some(20));
        assert_eq!(saved.video_led_pin, Some(21));
        assert_eq!(saved.hdmi_led_pin, Some(22));
    }
    #[test]
    fn migration_preserves_gpio_pins_and_defaults_conflicts() {
        let f = crate::update::tests::Fixture::new();
        let source = br#"{"beeper_pin":6,"power_button_pin":19,"beep_until_time_synced":true}"#;
        fs::write(f.0.join("settings.json"), source).unwrap();
        let output = f.0.join("migrated.json");
        let warnings = migrate(&f.0, &output).unwrap();
        assert!(warnings.is_empty());
        let migrated: Settings = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(migrated.beeper_pin, Some(6));
        assert!(migrated.beep_until_time_synced);
        assert!(!Settings::default().beep_until_time_synced);

        // A settings.json from a release that still had the LED feature
        // carries fields this schema no longer recognizes; migration should
        // report them as incompatible and fall back to defaults, not fail.
        let with_removed_led_fields =
            br#"{"beeper_pin":6,"led_pin":10,"led_low_bitrate_mbps":2.5,"led_brightness":45}"#;
        fs::write(f.0.join("settings.json"), with_removed_led_fields).unwrap();
        let warnings = migrate(&f.0, &output).unwrap();
        assert_eq!(warnings.len(), 3);
        let migrated: Settings = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(migrated.beeper_pin, Some(6));

        let conflicting = br#"{"beeper_pin":6,"power_button_pin":6}"#;
        fs::write(f.0.join("settings.json"), conflicting).unwrap();
        let warnings = migrate(&f.0, &output).unwrap();
        assert!(warnings.iter().any(|w| w.contains("conflicting")));
        let migrated: Settings = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(migrated.beeper_pin, Settings::default().beeper_pin);
        assert_eq!(
            migrated.power_button_pin,
            Settings::default().power_button_pin
        );
    }
    /// Every field of the current schema has to survive a migration to the
    /// same schema. `hdmi_connector` was once missing from `migrate`, so each
    /// update (and each switch back) reset a chosen HDMI output to automatic.
    /// The check below fails for any future field added without a `migrate` arm.
    #[test]
    fn migration_keeps_every_field_of_the_current_schema() {
        let f = crate::update::tests::Fixture::new();
        fs::create_dir(f.0.join("images")).unwrap();
        fs::write(f.0.join("images/custom.png"), "image").unwrap();
        let custom = Settings {
            hdmi_mode: "1920x1080@60".into(),
            hdmi_connector: Some(44),
            fallback_image: Some("custom.png".into()),
            preview_enabled: false,
            beeper_pin: Some(5),
            power_button_pin: Some(13),
            goggles_led_pin: Some(20),
            video_led_pin: Some(21),
            hdmi_led_pin: Some(22),
            beep_until_time_synced: true,
            stream_enabled: true,
            stream_quality: "event".into(),
            auto_record: true,
            crash_buffer: true,
            record_to_usb: true,
            wifi_always_ap: false,
            terminal_enabled: true,
            stretch_live: true,
            stretch_usb: true,
        };
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        let wanted = serde_json::to_value(&custom).unwrap();
        for (key, value) in wanted.as_object().unwrap() {
            assert_ne!(
                value, &defaults[key],
                "{key} equals its default in this test, so a missing migrate arm would go unnoticed"
            );
        }
        fs::write(
            f.0.join("settings.json"),
            serde_json::to_vec(&custom).unwrap(),
        )
        .unwrap();
        let output = f.0.join("migrated.json");
        assert!(migrate(&f.0, &output).unwrap().is_empty());
        let migrated: Settings = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(migrated, custom);

        fs::write(f.0.join("settings.json"), br#"{"hdmi_connector":null}"#).unwrap();
        assert!(migrate(&f.0, &output).unwrap().is_empty());
        let migrated: Settings = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(migrated.hdmi_connector, None);
        fs::write(f.0.join("settings.json"), br#"{"hdmi_connector":"x"}"#).unwrap();
        assert_eq!(migrate(&f.0, &output).unwrap().len(), 1);
    }
    #[test]
    fn terminal_is_off_by_default_and_survives_migration() {
        assert!(!Settings::default().terminal_enabled);
        let f = crate::update::tests::Fixture::new();
        fs::write(f.0.join("settings.json"), br#"{"terminal_enabled":true}"#).unwrap();
        let output = f.0.join("migrated.json");
        assert!(migrate(&f.0, &output).unwrap().is_empty());
        let migrated: Settings = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert!(migrated.terminal_enabled);
    }
    #[test]
    fn migration_preserves_images_and_reports_incompatible_values() {
        let f = crate::update::tests::Fixture::new();
        fs::create_dir(f.0.join("images")).unwrap();
        fs::write(f.0.join("images/custom.png"), "image").unwrap();
        let source =
            br#"{"hdmi_mode":"invalid","preview_enabled":false,"fallback_image":"custom.png"}"#;
        fs::write(f.0.join("settings.json"), source).unwrap();
        let output = f.0.join("migrated.json");
        let warnings = migrate(&f.0, &output).unwrap();
        assert_eq!(warnings.len(), 1);
        let migrated: Settings = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(migrated.hdmi_mode, "auto");
        assert!(!migrated.preview_enabled);
        assert_eq!(migrated.fallback_image.as_deref(), Some("custom.png"));
        assert_eq!(fs::read(f.0.join("settings.json")).unwrap(), source);
    }
}
