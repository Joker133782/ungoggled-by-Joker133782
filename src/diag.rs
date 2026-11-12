//! What the service writes to its log so that a problem can be understood
//! *afterwards*: the Pi is usually powered from the flight battery, so by the
//! time somebody looks, the session is long over.
//!
//! Everything goes to stderr, which systemd's journal keeps for 30 days (see
//! `scripts/prepare-pi.sh`). Three sources:
//!
//! * [`Watcher`]: every change of the receiver's own status (phase, goggles
//!   protocol, HDMI output, decoder starts) and every message the UI would
//!   show, one line each, rate-limited so a flapping stream cannot flood the
//!   log.
//! * [`Heartbeat`]: state that is only polled (USB controller state, HDMI
//!   display, clock, the Pi's under-voltage and temperature flags) as changes,
//!   plus a one-line summary every minute while goggles are involved.
//! * [`ControlLog`]: the first control packet of every kind the goggles send,
//!   which is what tells a Goggles 2 that answers from one that stays silent.
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// Status keys whose changes are logged.
const WATCHED: [&str; 10] = [
    "phase",
    "goggles_mode",
    "goggles_protocol",
    "goggles_protocol_state",
    "hdmi",
    "decoder_starts",
    "display_error",
    "power_button_error",
    "beeper_error",
    "buttons_error",
];
/// Per key: at most this many lines per window, then a count of the rest.
const BUDGET: u32 = 20;
const WINDOW: Duration = Duration::from_secs(60);
/// The same message is not logged twice within this time.
const MESSAGE_REPEAT: Duration = Duration::from_secs(10);

fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "none".into(),
        other => other.to_string(),
    }
}

/// At most [`BUDGET`] lines per [`WINDOW`]; what is left out is counted and
/// mentioned at the start of the next window.
struct Budget {
    start: Instant,
    used: u32,
    dropped: u32,
}
impl Budget {
    fn new(now: Instant) -> Self {
        Self {
            start: now,
            used: 0,
            dropped: 0,
        }
    }
    /// `Some(note)` (the note may be empty) when a line may be written now;
    /// `None` when this window's budget is used up.
    fn take(&mut self, now: Instant, what: &str) -> Option<Option<String>> {
        let mut note = None;
        if now.duration_since(self.start) >= WINDOW {
            if self.dropped > 0 {
                note = Some(format!(
                    "{what}: {} more changes in the last minute not listed",
                    self.dropped
                ));
            }
            *self = Self::new(now);
        }
        if self.used >= BUDGET {
            self.dropped += 1;
            return None;
        }
        self.used += 1;
        Some(note)
    }
}

#[derive(Default)]
pub struct Watcher {
    windows: HashMap<&'static str, Budget>,
    last_message: Option<(String, Instant)>,
}
impl Watcher {
    /// Log lines for the keys `event` changes compared with `old`.
    pub fn changes(
        &mut self,
        old: &Map<String, Value>,
        event: &Map<String, Value>,
        now: Instant,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        for key in WATCHED {
            let Some(new) = event.get(key) else {
                continue;
            };
            let before = old.get(key).unwrap_or(&Value::Null);
            if before == new {
                continue;
            }
            // An error field going back to null is worth a line too, but
            // "null -> null" never reaches here.
            let budget = self.windows.entry(key).or_insert_with(|| Budget::new(now));
            let Some(note) = budget.take(now, &format!("status {key}")) else {
                continue;
            };
            lines.extend(note);
            lines.push(format!("status {key}: {} -> {}", show(before), show(new)));
        }
        // A new receiver worker starts with "phase: starting": what it then
        // says is new again, even if the previous worker said the same.
        if event.get("phase").and_then(Value::as_str) == Some("starting") {
            self.last_message = None;
        }
        if let Some(Value::String(message)) = event.get("message") {
            let repeated = self
                .last_message
                .as_ref()
                .is_some_and(|(m, t)| m == message && now.duration_since(*t) < MESSAGE_REPEAT);
            if !message.is_empty() && !repeated {
                lines.push(format!("message: {message}"));
            }
            self.last_message = Some((message.clone(), now));
        }
        lines
    }
}

/// Decoded `vcgencmd get_throttled` bits that are true *now* (low nibble) and
/// that happened since boot (bits 16-19).
pub fn throttle_text(bits: u32) -> String {
    const NAMES: [&str; 4] = [
        "under-voltage",
        "ARM frequency capped",
        "throttled",
        "soft temperature limit",
    ];
    let part = |shift: u32| -> Vec<&str> {
        NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| bits >> (shift + *i as u32) & 1 == 1)
            .map(|(_, n)| *n)
            .collect()
    };
    let (now, since) = (part(0), part(16));
    let join = |v: Vec<&str>| {
        if v.is_empty() {
            "none".to_owned()
        } else {
            v.join(", ")
        }
    };
    format!("now: {}; since boot: {}", join(now), join(since))
}

/// `vcgencmd get_throttled` -> bits. `None` where the tool does not exist.
pub fn read_throttled() -> Option<u32> {
    let out = std::process::Command::new("timeout")
        .args(["5", "vcgencmd", "get_throttled"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let hex = text.trim().strip_prefix("throttled=0x")?;
    u32::from_str_radix(hex, 16).ok()
}

#[derive(Default)]
pub struct Heartbeat {
    changes: Option<Budget>,
    last: Option<Instant>,
    usb: Option<Value>,
    hdmi: Option<bool>,
    synced: Option<bool>,
    throttled: Option<u32>,
}
impl Heartbeat {
    /// Called with every status snapshot (once a second). `throttled` is the
    /// latest `vcgencmd get_throttled` reading, if there is one.
    pub fn observe(&mut self, s: &Value, throttled: Option<u32>, now: Instant) -> Vec<String> {
        let mut lines = Vec::new();
        // A cable that flaps must not flood the log: the state is still
        // tracked, only the lines are limited.
        let budget = self.changes.get_or_insert_with(|| Budget::new(now));
        let mut change = |text: String| {
            if let Some(note) = budget.take(now, "state") {
                lines.extend(note);
                lines.push(text);
            }
        };
        let usb = s.get("usb_state").cloned().unwrap_or(Value::Null);
        if self.usb.as_ref() != Some(&usb) {
            change(format!(
                "usb_state: {} -> {}",
                self.usb.as_ref().map_or("unknown".into(), show),
                show(&usb)
            ));
            self.usb = Some(usb.clone());
        }
        let hdmi = s["hdmi_connected"].as_bool().unwrap_or(false);
        if self.hdmi != Some(hdmi) {
            change(format!("hdmi display connected: {hdmi}"));
            self.hdmi = Some(hdmi);
        }
        let synced = s["time_synced"].as_bool().unwrap_or(false);
        if self.synced != Some(synced) {
            change(format!(
                "clock synced: {synced} (the time of these lines is only right when true)"
            ));
            self.synced = Some(synced);
        }
        if let Some(bits) = throttled {
            // What happened since boot only accumulates; log the first sighting
            // and every change of either half.
            if self.throttled != Some(bits) {
                change(format!(
                    "power/thermal: {} (0x{bits:x})",
                    throttle_text(bits)
                ));
                self.throttled = Some(bits);
            }
        }
        let attached = usb.as_str().is_some_and(|u| u != "not attached");
        let busy = attached || s["phase"].as_str() == Some("streaming");
        let every = Duration::from_secs(if busy { 60 } else { 900 });
        if self.last.is_none_or(|t| now.duration_since(t) >= every) {
            self.last = Some(now);
            lines.push(Self::summary(s, throttled));
        }
        lines
    }
    fn summary(s: &Value, throttled: Option<u32>) -> String {
        let num = |key: &str| {
            s[key]
                .as_f64()
                .map_or("-".to_owned(), |v| format!("{v:.1}"))
        };
        let int = |key: &str| s[key].as_u64().map_or("-".to_owned(), |v| v.to_string());
        let text = |key: &str| s[key].as_str().unwrap_or("-").to_owned();
        format!(
            "heartbeat: phase={} usb={} protocol={}/{} video={}Mbps in_fps={} out_fps={} decoder_starts={} \
             discarded_bytes={} control_packets={} hdmi={} temp={}C throttled={} up={}s",
            text("phase"),
            text("usb_state"),
            text("goggles_protocol"),
            text("goggles_protocol_state"),
            num("bitrate_mbps"),
            num("input_fps"),
            num("output_fps"),
            int("decoder_starts"),
            int("discarded_bytes"),
            int("control_packets"),
            text("hdmi"),
            num("temperature_c"),
            throttled.map_or("-".to_owned(), |b| format!("0x{b:x}")),
            int("uptime_seconds"),
        )
    }
}

/// First packet of every kind (port, sender, receiver, command set, command,
/// flags) a session sees, up to a limit.
#[derive(Default)]
pub struct ControlLog {
    seen: HashSet<(u16, u8, u8, u8, u8, u8)>,
}
const KINDS: usize = 120;
const PAYLOAD_SHOWN: usize = 32;
impl ControlLog {
    /// `duml` must have passed `protocol::valid`.
    pub fn first_of_kind(&mut self, port: u16, duml: &[u8]) -> Option<String> {
        let key = (port, duml[4], duml[5], duml[9], duml[10], duml[8]);
        if self.seen.len() >= KINDS || !self.seen.insert(key) {
            return None;
        }
        let payload = &duml[11..duml.len() - 2];
        let shown: String = payload
            .iter()
            .take(PAYLOAD_SHOWN)
            .map(|b| format!("{b:02x}"))
            .collect();
        let more = if payload.len() > PAYLOAD_SHOWN {
            format!("...(+{} bytes)", payload.len() - PAYLOAD_SHOWN)
        } else {
            String::new()
        };
        Some(format!(
            "DUML port 0x{port:04x} {:02x}->{:02x} {:02x}:{:02x} flags={:02x} payload={shown}{more}",
            duml[4], duml[5], duml[9], duml[10], duml[8]
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn changes_of_watched_keys_are_logged_once() {
        let mut w = Watcher::default();
        let t = Instant::now();
        let old = map(json!({"phase":"waiting_video"}));
        let lines = w.changes(&old, &map(json!({"phase":"streaming","video_bytes":5})), t);
        assert_eq!(lines, ["status phase: waiting_video -> streaming"]);
        assert!(
            w.changes(
                &map(json!({"phase":"streaming"})),
                &map(json!({"phase":"streaming"})),
                t
            )
            .is_empty()
        );
        assert!(
            w.changes(&old, &map(json!({"video_bytes":9,"bitrate_mbps":1.5})), t)
                .is_empty()
        );
    }

    #[test]
    fn a_flapping_key_cannot_flood_the_log() {
        let mut w = Watcher::default();
        let t = Instant::now();
        let mut logged = 0;
        for i in 0..200 {
            let (a, b) = if i % 2 == 0 {
                ("waiting_video", "streaming")
            } else {
                ("streaming", "waiting_video")
            };
            logged += w
                .changes(&map(json!({"phase":a})), &map(json!({"phase":b})), t)
                .len();
        }
        assert_eq!(logged, BUDGET as usize);
        // The next minute starts with a note about what was left out.
        let lines = w.changes(
            &map(json!({"phase":"streaming"})),
            &map(json!({"phase":"waiting_video"})),
            t + WINDOW,
        );
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("180 more changes"), "{lines:?}");
    }

    #[test]
    fn messages_are_logged_but_not_repeated_in_a_burst() {
        let mut w = Watcher::default();
        let t = Instant::now();
        let e = map(json!({"message":"Video queue overrun; recovering at next keyframe"}));
        assert_eq!(w.changes(&Map::new(), &e, t).len(), 1);
        assert!(
            w.changes(&Map::new(), &e, t + Duration::from_secs(2))
                .is_empty()
        );
        assert_eq!(
            w.changes(&Map::new(), &e, t + Duration::from_secs(30))
                .len(),
            1
        );
        assert!(
            w.changes(&Map::new(), &map(json!({"message":""})), t)
                .is_empty()
        );
    }

    #[test]
    fn a_message_repeated_after_a_worker_restart_is_logged_again() {
        let mut w = Watcher::default();
        let t = Instant::now();
        let ready = map(json!({"message":"FunctionFS ready for goggles"}));
        assert_eq!(w.changes(&Map::new(), &ready, t).len(), 1);
        assert!(
            w.changes(&Map::new(), &ready, t + Duration::from_secs(2))
                .is_empty()
        );
        w.changes(
            &Map::new(),
            &map(json!({"phase":"starting"})),
            t + Duration::from_secs(3),
        );
        assert_eq!(
            w.changes(&Map::new(), &ready, t + Duration::from_secs(5))
                .len(),
            1
        );
    }

    #[test]
    fn throttle_bits_are_spelled_out() {
        assert_eq!(throttle_text(0), "now: none; since boot: none");
        assert_eq!(
            throttle_text(0x50005),
            "now: under-voltage, throttled; since boot: under-voltage, throttled"
        );
        assert_eq!(
            throttle_text(0x80000),
            "now: none; since boot: soft temperature limit"
        );
    }

    #[test]
    fn heartbeat_logs_changes_at_once_and_the_summary_on_schedule() {
        let mut h = Heartbeat::default();
        let t = Instant::now();
        let snapshot = |usb: Value| {
            json!({"usb_state":usb,"hdmi_connected":true,"time_synced":false,"phase":"waiting_video",
                   "bitrate_mbps":0.0,"temperature_c":51.5,"uptime_seconds":12})
        };
        let first = h.observe(&snapshot(json!("configured")), Some(0), t);
        assert!(
            first
                .iter()
                .any(|l| l == "usb_state: unknown -> configured"),
            "{first:?}"
        );
        assert!(
            first
                .iter()
                .any(|l| l.starts_with("heartbeat: phase=waiting_video usb=configured")),
            "{first:?}"
        );
        assert!(
            first
                .iter()
                .any(|l| l.starts_with("power/thermal: now: none")),
            "{first:?}"
        );
        // Nothing changed one second later.
        assert!(
            h.observe(
                &snapshot(json!("configured")),
                Some(0),
                t + Duration::from_secs(1)
            )
            .is_empty()
        );
        // A change is logged immediately...
        let lines = h.observe(
            &snapshot(json!("not attached")),
            Some(0),
            t + Duration::from_secs(2),
        );
        assert_eq!(lines, ["usb_state: configured -> not attached"]);
        // ...and with goggles attached the summary comes every minute.
        let lines = h.observe(
            &snapshot(json!("configured")),
            Some(0),
            t + Duration::from_secs(61),
        );
        assert!(
            lines.iter().any(|l| l.starts_with("heartbeat:")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_flapping_usb_cable_cannot_flood_the_log() {
        let mut h = Heartbeat::default();
        let t = Instant::now();
        let snapshot = |usb: &str| json!({"usb_state":usb,"hdmi_connected":true,"time_synced":true,"phase":"stopped"});
        let mut changes = 0;
        for i in 0..200u64 {
            let usb = if i % 2 == 0 {
                "configured"
            } else {
                "not attached"
            };
            changes += h
                .observe(&snapshot(usb), Some(0), t + Duration::from_millis(i * 100))
                .iter()
                .filter(|l| !l.starts_with("heartbeat:"))
                .count();
        }
        assert_eq!(
            changes, BUDGET as usize,
            "20 seconds of flapping: the budget"
        );
        // A minute later the note says how many were left out, and logging resumes.
        let lines = h.observe(
            &snapshot("configured"),
            Some(0),
            t + WINDOW + Duration::from_secs(1),
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("more changes in the last minute")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("usb_state:")),
            "{lines:?}"
        );
    }

    #[test]
    fn idle_receiver_logs_a_summary_only_every_quarter_hour() {
        let mut h = Heartbeat::default();
        let t = Instant::now();
        let idle = json!({"usb_state":"not attached","hdmi_connected":false,"time_synced":true,"phase":"stopped"});
        assert!(
            h.observe(&idle, None, t)
                .iter()
                .any(|l| l.starts_with("heartbeat:"))
        );
        assert!(
            h.observe(&idle, None, t + Duration::from_secs(120))
                .is_empty()
        );
        assert!(
            h.observe(&idle, None, t + Duration::from_secs(901))
                .iter()
                .any(|l| l.starts_with("heartbeat:"))
        );
    }

    #[test]
    fn control_packets_are_listed_once_per_kind() {
        let mut log = ControlLog::default();
        let p = crate::protocol::duml(2, 0x3c, 0x101, 0x40, 0, 0x88, &[0x19, 0x00]);
        let line = log.first_of_kind(0x5749, &p).unwrap();
        assert_eq!(line, "DUML port 0x5749 02->3c 00:88 flags=40 payload=1900");
        assert!(log.first_of_kind(0x5749, &p).is_none(), "same kind again");
        assert!(log.first_of_kind(0x7530, &p).is_some(), "other port");
        let long = crate::protocol::duml(2, 0x3c, 0x102, 0x40, 0, 0x99, &[7; 40]);
        assert!(
            log.first_of_kind(0x5749, &long)
                .unwrap()
                .ends_with("(+8 bytes)")
        );
    }

    #[test]
    fn the_number_of_kinds_is_bounded() {
        let mut log = ControlLog::default();
        let mut listed = 0;
        for id in 0..=255u8 {
            let p = crate::protocol::duml(2, 0x3c, 1, 0x40, 0, id, &[]);
            listed += usize::from(log.first_of_kind(0x7530, &p).is_some());
        }
        assert_eq!(listed, KINDS);
    }
}
