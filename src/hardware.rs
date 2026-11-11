//! Piezo beeper, status LEDs, the power/shutdown button and up to four more
//! momentary switches with a configurable action each.
//!
//! Default BCM pins: piezo GPIO6, shutdown button GPIO19 (NuclearHazard V5 hat,
//! active low), goggles LED GPIO17, video LED GPIO23, HDMI LED GPIO24. The hat's
//! own beeper is soldered to its onboard STM32 co-processor, so GPIO6 assumes a
//! separately wired piezo.
use crate::settings::{GpioButton, Store};
use rppal::gpio::{Gpio, InputPin, OutputPin};
use serde_json::Value;
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const DEFAULT_BEEPER_PIN: u8 = 6;
pub const DEFAULT_POWER_BUTTON_PIN: u8 = 19;
pub const DEFAULT_GOGGLES_LED_PIN: u8 = 17;
pub const DEFAULT_VIDEO_LED_PIN: u8 = 23;
pub const DEFAULT_HDMI_LED_PIN: u8 = 24;

pub type StatusFn = Arc<dyn Fn() -> Value + Send + Sync>;
/// Carries out what a button is set to do; the result (what happened, or why
/// not) goes to the log. Runs on its own thread: some actions take seconds.
pub type ButtonFn = Arc<dyn Fn(&GpioButton) -> Result<String, String> + Send + Sync>;

#[derive(Clone, Copy)]
struct Tone {
    hz: u32,
    on_ms: u32,
    off_ms: u32,
}
type Pattern = &'static [Tone];

const BOOT: Pattern = &[
    Tone {
        hz: 988,
        on_ms: 70,
        off_ms: 25,
    },
    Tone {
        hz: 1245,
        on_ms: 70,
        off_ms: 25,
    },
    Tone {
        hz: 1568,
        on_ms: 130,
        off_ms: 0,
    },
];
const GOGGLES_UP: Pattern = &[
    Tone {
        hz: 988,
        on_ms: 60,
        off_ms: 35,
    },
    Tone {
        hz: 1319,
        on_ms: 100,
        off_ms: 0,
    },
];
const GOGGLES_DOWN: Pattern = &[
    Tone {
        hz: 988,
        on_ms: 80,
        off_ms: 35,
    },
    Tone {
        hz: 659,
        on_ms: 140,
        off_ms: 0,
    },
];
const VIDEO_UP: Pattern = &[Tone {
    hz: 1568,
    on_ms: 140,
    off_ms: 0,
}];
const VIDEO_DOWN: Pattern = &[Tone {
    hz: 494,
    on_ms: 200,
    off_ms: 0,
}];
const HDMI_UP: Pattern = &[
    Tone {
        hz: 1245,
        on_ms: 55,
        off_ms: 35,
    },
    Tone {
        hz: 1245,
        on_ms: 55,
        off_ms: 35,
    },
    Tone {
        hz: 1568,
        on_ms: 90,
        off_ms: 0,
    },
];
const HDMI_DOWN: Pattern = &[
    Tone {
        hz: 784,
        on_ms: 55,
        off_ms: 35,
    },
    Tone {
        hz: 784,
        on_ms: 55,
        off_ms: 35,
    },
    Tone {
        hz: 392,
        on_ms: 130,
        off_ms: 0,
    },
];
const BUTTON_PRESSED: Pattern = &[Tone {
    hz: 1200,
    on_ms: 120,
    off_ms: 0,
}];
const TIME_UNSYNCED: Pattern = &[Tone {
    hz: 2093,
    on_ms: 40,
    off_ms: 0,
}];
const TIME_UNSYNCED_INTERVAL: Duration = Duration::from_secs(5);
const TEST: Pattern = &[
    Tone {
        hz: 880,
        on_ms: 120,
        off_ms: 70,
    },
    Tone {
        hz: 880,
        on_ms: 120,
        off_ms: 0,
    },
];

// Phases the gadget only reaches once the accessory handshake has succeeded.
// Mirrors web/src/App.tsx's `usbPhases` so both sides agree on "attached".
const USB_PHASES: [&str; 5] = [
    "usb_connected",
    "accessory",
    "aoa_negotiation",
    "waiting_video",
    "streaming",
];
fn goggles_attached(status: &Value) -> bool {
    match status["usb_state"].as_str() {
        Some(state) => state != "not attached",
        None => status["phase"]
            .as_str()
            .is_some_and(|p| USB_PHASES.contains(&p)),
    }
}
fn video_live(status: &Value) -> bool {
    status["phase"].as_str() == Some("streaming")
        && status["bitrate_mbps"].as_f64().unwrap_or(0.0) > 0.0
}
fn hdmi_connected(status: &Value) -> bool {
    status["hdmi_connected"].as_bool().unwrap_or(false)
}
fn time_synced(status: &Value) -> bool {
    status["time_synced"].as_bool().unwrap_or(false)
}

// Opens (or drops) a pin when the desired GPIO changes, reporting the
// outcome under `error_key` in the shared status so failures are visible in
// the web UI without needing a shell on the Pi.
fn reconfigure<T>(
    wanted: Option<u8>,
    open: impl FnOnce(u8) -> Result<T, String>,
    report: &crate::video::Report,
    error_key: &str,
) -> Option<T> {
    let (value, error) = match wanted.map(open) {
        None => (None, Value::Null),
        Some(Ok(v)) => (Some(v), Value::Null),
        Some(Err(e)) => (None, Value::String(e)),
    };
    let mut event = serde_json::Map::new();
    event.insert(error_key.to_string(), error);
    report(Value::Object(event));
    value
}

pub fn start(
    status: StatusFn,
    store: Store,
    shutdown: Arc<AtomicBool>,
    test_beep: Arc<AtomicBool>,
    report: crate::video::Report,
    run_button: ButtonFn,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Ok(gpio) = Gpio::new() else {
            eprintln!("GPIO unavailable; beeper, LEDs and power button are disabled this run");
            for key in [
                "beeper_error",
                "power_button_error",
                "goggles_led_error",
                "video_led_error",
                "hdmi_led_error",
                "buttons_error",
            ] {
                let mut event = serde_json::Map::new();
                event.insert(key.to_string(), Value::String("GPIO unavailable".into()));
                report(Value::Object(event));
            }
            return;
        };
        let mut beeper_want: Option<u8> = None;
        let mut beeper: Option<OutputPin> = None;
        let mut button_want: Option<u8> = None;
        let mut button: Option<InputPin> = None;
        let mut goggles_led_want: Option<u8> = None;
        let mut goggles_led: Option<OutputPin> = None;
        let mut video_led_want: Option<u8> = None;
        let mut video_led: Option<OutputPin> = None;
        let mut hdmi_led_want: Option<u8> = None;
        let mut hdmi_led: Option<OutputPin> = None;
        let mut prev_goggles: Option<bool> = None;
        let mut prev_video: Option<bool> = None;
        let mut prev_hdmi: Option<bool> = None;
        let mut press = PressTracker::default();
        let mut buttons_want: Vec<GpioButton> = Vec::new();
        let mut buttons: Vec<ExtraButton> = Vec::new();
        let mut shutdown_triggered = false;
        let mut booted = false;
        let mut last_unsynced_beep = Instant::now();
        while !shutdown.load(Ordering::Relaxed) {
            let settings = store.get();
            if settings.beeper_pin != beeper_want {
                beeper_want = settings.beeper_pin;
                beeper = reconfigure(
                    beeper_want,
                    |p| open_output(&gpio, p, "beeper"),
                    &report,
                    "beeper_error",
                );
            }
            if settings.power_button_pin != button_want {
                button_want = settings.power_button_pin;
                button = reconfigure(
                    button_want,
                    |p| open_input_pullup(&gpio, p, "power button"),
                    &report,
                    "power_button_error",
                );
                press = PressTracker::default();
            }
            if settings.buttons != buttons_want {
                buttons_want = settings.buttons.clone();
                // Let go of the old pins first: a pin cannot be claimed twice.
                buttons.clear();
                let mut errors = Vec::new();
                for config in &buttons_want {
                    match open_input_pullup(&gpio, config.pin, "button") {
                        Ok(pin) => buttons.push(ExtraButton {
                            config: config.clone(),
                            pin,
                            press: PressTracker::holding(config.hold_ms),
                        }),
                        Err(e) => errors.push(e),
                    }
                }
                let mut event = serde_json::Map::new();
                event.insert(
                    "buttons_error".into(),
                    if errors.is_empty() {
                        Value::Null
                    } else {
                        Value::String(errors.join("; "))
                    },
                );
                report(Value::Object(event));
            }
            if settings.goggles_led_pin != goggles_led_want {
                goggles_led_want = settings.goggles_led_pin;
                goggles_led = reconfigure(
                    goggles_led_want,
                    |p| open_output(&gpio, p, "goggles LED"),
                    &report,
                    "goggles_led_error",
                );
            }
            if settings.video_led_pin != video_led_want {
                video_led_want = settings.video_led_pin;
                video_led = reconfigure(
                    video_led_want,
                    |p| open_output(&gpio, p, "video LED"),
                    &report,
                    "video_led_error",
                );
            }
            if settings.hdmi_led_pin != hdmi_led_want {
                hdmi_led_want = settings.hdmi_led_pin;
                hdmi_led = reconfigure(
                    hdmi_led_want,
                    |p| open_output(&gpio, p, "HDMI LED"),
                    &report,
                    "hdmi_led_error",
                );
            }
            if !booted {
                booted = true;
                if let Some(pin) = &mut beeper {
                    play(pin, BOOT);
                }
            }
            if test_beep.swap(false, Ordering::Relaxed)
                && let Some(pin) = &mut beeper
            {
                play(pin, TEST);
            }

            let s = status();
            let goggles = goggles_attached(&s);
            let video = video_live(&s);
            let hdmi = hdmi_connected(&s);
            if let Some(pin) = &mut beeper {
                if prev_goggles.is_some_and(|prev| prev != goggles) {
                    play(pin, if goggles { GOGGLES_UP } else { GOGGLES_DOWN });
                }
                if prev_video.is_some_and(|prev| prev != video) {
                    play(pin, if video { VIDEO_UP } else { VIDEO_DOWN });
                }
                if prev_hdmi.is_some_and(|prev| prev != hdmi) {
                    play(pin, if hdmi { HDMI_UP } else { HDMI_DOWN });
                }
            }
            if settings.beep_until_time_synced && !time_synced(&s) {
                if last_unsynced_beep.elapsed() >= TIME_UNSYNCED_INTERVAL {
                    last_unsynced_beep = Instant::now();
                    if let Some(pin) = &mut beeper {
                        play(pin, TIME_UNSYNCED);
                    }
                }
            } else {
                last_unsynced_beep = Instant::now();
            }
            prev_goggles = Some(goggles);
            prev_video = Some(video);
            prev_hdmi = Some(hdmi);

            for (pin, on) in [
                (&mut goggles_led, goggles),
                (&mut video_led, video),
                (&mut hdmi_led, hdmi),
            ] {
                if let Some(pin) = pin {
                    if on {
                        pin.set_high();
                    } else {
                        pin.set_low();
                    }
                }
            }

            if let Some(pin) = &button
                && press.tick(pin.is_low())
                && !shutdown_triggered
            {
                shutdown_triggered = true;
                if let Some(pin) = &mut beeper {
                    play(pin, BUTTON_PRESSED);
                }
                let _ = Command::new("systemctl").arg("poweroff").spawn();
            }
            for button in &mut buttons {
                if button.press.tick(button.pin.is_low()) {
                    if let Some(pin) = &mut beeper {
                        play(pin, BUTTON_PRESSED);
                    }
                    let config = button.config.clone();
                    let run = run_button.clone();
                    thread::spawn(move || match run(&config) {
                        Ok(done) => {
                            eprintln!("Button on GPIO{} ({:?}): {done}", config.pin, config.action)
                        }
                        Err(why) => eprintln!(
                            "Button on GPIO{} ({:?}) did nothing: {why}",
                            config.pin, config.action
                        ),
                    });
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    })
}
/// Runs `button-scripts/<name>` from the data directory (root-owned; only
/// someone with a shell on the Pi can put a file there, the settings can only
/// name one) and waits for it, one minute at most.
pub fn run_script(data_dir: &std::path::Path, name: &str) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    if !crate::settings::valid_script_name(name) {
        return Err(format!("{name:?} is not a plain file name"));
    }
    let path = data_dir.join("button-scripts").join(name);
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(format!("{} is not executable (chmod +x)", path.display()));
    }
    let status = Command::new("timeout")
        .arg("60")
        .arg(&path)
        .current_dir(data_dir)
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    match status.code() {
        Some(0) => Ok(format!("script {name} finished")),
        Some(124) => Err(format!("script {name} was stopped after 60 seconds")),
        _ => Err(format!("script {name} failed: {status}")),
    }
}
struct ExtraButton {
    config: GpioButton,
    pin: InputPin,
    press: PressTracker,
}
/// Turns the power button's readings (one per 100 ms) into "pressed".
///
/// The button only counts once it has been seen released: a pin that reads
/// low from the very start is not a button being held but a wrong pin
/// number (a HAT signal or another device driving it low), and acting on it
/// would power the Pi off 0.3 s after every start, with the console never
/// reachable long enough to correct the setting.
struct PressTracker {
    armed: bool,
    low_ticks: u8,
    /// Low readings in a row that make a press.
    needed: u8,
}
impl Default for PressTracker {
    fn default() -> Self {
        Self {
            armed: false,
            low_ticks: 0,
            needed: 3,
        }
    }
}
impl PressTracker {
    /// A button that has to be held for `hold_ms`; never less than the three
    /// readings (0.3 s) that keep a contact bounce from counting.
    fn holding(hold_ms: u32) -> Self {
        Self {
            needed: (hold_ms / 100).clamp(3, 200) as u8,
            ..Self::default()
        }
    }
    /// `true` on the reading that completes a press: the needed number of low
    /// readings in a row (three for a tap) after the button was last seen high.
    /// Only once per press: holding on does not repeat it.
    fn tick(&mut self, low: bool) -> bool {
        if !low {
            self.armed = true;
            self.low_ticks = 0;
            return false;
        }
        if !self.armed {
            return false;
        }
        self.low_ticks = self.low_ticks.saturating_add(1);
        self.low_ticks == self.needed
    }
}
fn open_output(gpio: &Gpio, pin: u8, name: &str) -> Result<OutputPin, String> {
    match gpio.get(pin) {
        Ok(p) => {
            let mut out = p.into_output();
            out.set_low();
            Ok(out)
        }
        Err(e) => {
            let msg = format!("Cannot claim GPIO{pin} for the {name}: {e}");
            eprintln!("{msg}");
            Err(msg)
        }
    }
}
fn open_input_pullup(gpio: &Gpio, pin: u8, name: &str) -> Result<InputPin, String> {
    match gpio.get(pin) {
        Ok(p) => Ok(p.into_input_pullup()),
        Err(e) => {
            let msg = format!("Cannot claim GPIO{pin} for the {name}: {e}");
            eprintln!("{msg}");
            Err(msg)
        }
    }
}
fn play(pin: &mut OutputPin, pattern: Pattern) {
    for tone in pattern {
        square_wave(pin, tone.hz, tone.on_ms);
        pin.set_low();
        if tone.off_ms > 0 {
            thread::sleep(Duration::from_millis(tone.off_ms as u64));
        }
    }
}
fn square_wave(pin: &mut OutputPin, hz: u32, ms: u32) {
    let half = Duration::from_secs_f64(0.5 / hz.max(1) as f64);
    let end = Instant::now() + Duration::from_millis(ms as u64);
    let mut high = false;
    while Instant::now() < end {
        high = !high;
        if high {
            pin.set_high();
        } else {
            pin.set_low();
        }
        thread::sleep(half);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn a_press_is_three_low_readings_after_the_button_was_released() {
        let mut p = PressTracker::default();
        assert!(!p.tick(false));
        assert!(!p.tick(true));
        assert!(!p.tick(true));
        assert!(p.tick(true), "third reading");
        assert!(!p.tick(true), "only once per press");
    }

    #[test]
    fn a_short_blip_is_not_a_press() {
        let mut p = PressTracker::default();
        p.tick(false);
        assert!(!p.tick(true));
        assert!(!p.tick(true));
        assert!(!p.tick(false));
        assert!(!p.tick(true));
        assert!(!p.tick(true));
        assert!(p.tick(true), "the count starts over after a release");
    }

    #[test]
    fn a_held_button_counts_only_after_its_hold_time_and_only_once() {
        let mut p = PressTracker::holding(2000);
        p.tick(false);
        for i in 1..=19 {
            assert!(!p.tick(true), "reading {i}");
        }
        assert!(p.tick(true), "the 20th reading is two seconds");
        for _ in 0..50 {
            assert!(!p.tick(true), "holding on does not repeat it");
        }
        // Let go early next time: nothing.
        p.tick(false);
        for _ in 0..19 {
            assert!(!p.tick(true));
        }
        assert!(!p.tick(false));
        assert!(!p.tick(true));
    }

    #[test]
    fn scripts_run_from_their_directory_only_when_they_are_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ungoggled-scripts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let scripts = dir.join("button-scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let write = |name: &str, body: &str, mode: u32| {
            let path = scripts.join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        write("ok.sh", "#!/bin/sh\ntouch ran-ok\n", 0o755);
        write("fails.sh", "#!/bin/sh\nexit 3\n", 0o755);
        write("plain.sh", "#!/bin/sh\ntrue\n", 0o644);
        assert!(run_script(&dir, "ok.sh").unwrap().contains("finished"));
        assert!(dir.join("ran-ok").exists(), "ran inside the data directory");
        assert!(run_script(&dir, "fails.sh").unwrap_err().contains("failed"));
        assert!(
            run_script(&dir, "plain.sh")
                .unwrap_err()
                .contains("not executable")
        );
        assert!(run_script(&dir, "missing.sh").is_err());
        for bad in ["../x", "a/b", ".hidden", ""] {
            assert!(run_script(&dir, bad).is_err(), "{bad:?}");
        }
        std::fs::create_dir(scripts.join("folder")).unwrap();
        assert!(run_script(&dir, "folder").is_err());
    }

    #[test]
    fn short_and_absurd_hold_times_are_kept_in_range() {
        assert_eq!(PressTracker::holding(0).needed, 3);
        assert_eq!(PressTracker::holding(100).needed, 3);
        assert_eq!(PressTracker::holding(10_000).needed, 100);
        assert_eq!(PressTracker::holding(u32::MAX).needed, 200);
    }

    #[test]
    fn a_pin_that_is_low_from_the_start_never_powers_the_pi_off() {
        let mut p = PressTracker::default();
        for _ in 0..1000 {
            assert!(!p.tick(true));
        }
        // Once it has been released, it is a working button.
        p.tick(false);
        p.tick(true);
        p.tick(true);
        assert!(p.tick(true));
    }

    #[test]
    fn derives_states_from_the_same_fields_the_web_header_uses() {
        assert!(goggles_attached(&json!({"usb_state":"configured"})));
        assert!(!goggles_attached(&json!({"usb_state":"not attached"})));
        assert!(goggles_attached(&json!({"phase":"streaming"})));
        assert!(!goggles_attached(&json!({"phase":"stopped"})));
        assert!(video_live(&json!({"phase":"streaming","bitrate_mbps":5.0})));
        assert!(!video_live(
            &json!({"phase":"streaming","bitrate_mbps":0.0})
        ));
        assert!(!video_live(
            &json!({"phase":"waiting_video","bitrate_mbps":5.0})
        ));
        assert!(hdmi_connected(&json!({"hdmi_connected":true})));
        assert!(!hdmi_connected(&json!({})));
        assert!(time_synced(&json!({"time_synced":true})));
        assert!(!time_synced(&json!({})));
    }
}
