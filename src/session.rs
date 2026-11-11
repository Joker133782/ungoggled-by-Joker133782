//! Which goggles protocol drives the accessory session.
//!
//! Goggles 3 start video after two mobile registrations on the configured
//! control port. Goggles 2 ignore those and need DJI Fly's arming burst plus
//! keepalives on 0x7530 (see `g2.rs`). The goggles do not identify their
//! generation in a documented way before video, so in `auto` mode the session
//! tries one protocol and switches to the other while no video arrives. The
//! protocol that produced video is kept for the rest of the session.
//!
//! Across connections, the receiver remembers which protocol worked for each
//! set of AOA identification strings the goggles sent while connecting. Strings
//! that only ever produced video with one protocol select it and *stay* with
//! it: goggles that have shown video once are never probed with the other
//! generation's requests again (a Goggles 3 whose drone is simply not switched
//! on yet must not be sent the Goggles 2 arming burst). Unknown strings, or
//! strings seen with both generations, start with the Goggles 3 probe; the
//! probe phases of an unknown pair get longer and longer, so goggles without
//! a drone are not hit with an arming burst every twenty seconds.
use crate::{clock, g2, protocol};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::PathBuf,
    sync::atomic::{AtomicU8, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Goggles {
    G3 = 0,
    G2 = 1,
}
impl Goggles {
    pub fn id(self) -> &'static str {
        match self {
            Self::G3 => "goggles3",
            Self::G2 => "goggles2",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::G3 => "Goggles 3",
            Self::G2 => "Goggles 2",
        }
    }
    fn other(self) -> Self {
        match self {
            Self::G3 => Self::G2,
            Self::G2 => Self::G3,
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        [Self::G3, Self::G2]
            .into_iter()
            .find(|g| g.id() == id.trim())
    }
}

/// State shared with the USB receive thread.
#[derive(Default)]
pub struct Shared {
    profile: AtomicU8,
    /// Video payload bytes received so far in this session.
    pub video: AtomicU64,
}
impl Shared {
    pub fn profile(&self) -> Goggles {
        if self.profile.load(Ordering::Relaxed) == Goggles::G2 as u8 {
            Goggles::G2
        } else {
            Goggles::G3
        }
    }
    /// Control port the goggles' requests are answered on.
    pub fn reply_port(&self, g3_port: u16) -> u16 {
        match self.profile() {
            Goggles::G3 => g3_port,
            Goggles::G2 => g2::PORT,
        }
    }
}

pub struct Config {
    /// `auto`, `goggles3` or `goggles2`.
    pub mode: String,
    pub g3_port: u16,
    pub sync_time: bool,
    /// Learned protocol per goggles identification (auto mode only).
    pub memory: Option<PathBuf>,
    /// AOA identification strings the goggles sent while connecting.
    pub aoa: Vec<String>,
}

/// Learned protocols, keyed by a hash of the AOA identification strings.
#[derive(Default, Serialize, Deserialize)]
struct Memory {
    /// Oldest first; bounded so the file stays small.
    goggles: Vec<Learned>,
}
#[derive(Serialize, Deserialize)]
struct Learned {
    id: String,
    /// Protocols that produced video for this identification.
    protocols: Vec<String>,
}
const MEMORY_LIMIT: usize = 32;

impl Memory {
    fn load(path: &std::path::Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    /// The protocol to try first, when this identification only ever worked
    /// with one of them.
    fn preferred(&self, id: &str) -> Option<Goggles> {
        let entry = self.goggles.iter().find(|e| e.id == id)?;
        match entry.protocols.as_slice() {
            [only] => Goggles::from_id(only),
            _ => None,
        }
    }
    /// Record a success; returns whether anything changed.
    fn learn(&mut self, id: &str, g: Goggles) -> bool {
        if let Some(entry) = self.goggles.iter_mut().find(|e| e.id == id) {
            if entry.protocols.iter().any(|p| p == g.id()) {
                return false;
            }
            entry.protocols.push(g.id().into());
            return true;
        }
        self.goggles.push(Learned {
            id: id.into(),
            protocols: vec![g.id().into()],
        });
        if self.goggles.len() > MEMORY_LIMIT {
            self.goggles.remove(0);
        }
        true
    }
}

/// Stable identification of the goggles from their AOA strings, if any.
pub fn identification(aoa: &[String]) -> Option<String> {
    if aoa.iter().all(|s| s.is_empty()) {
        return None;
    }
    let digest = Sha256::digest(aoa.join("\u{1f}").as_bytes());
    Some(crate::update::package::hex(&digest[..8]))
}

pub struct Timing {
    /// How long to wait for video before switching away from each protocol.
    /// The Goggles 3 phase doubles after every round without video...
    pub g3_probe: Duration,
    /// ...up to this.
    pub g3_probe_max: Duration,
    /// Includes the ~3.3 s Goggles 2 arming burst.
    pub g2_probe: Duration,
    pub g3_period: Duration,
    pub arm_gap_cap: Duration,
    /// Optional cap on keepalive gaps (tests only; `None` keeps the ~1 s loop).
    pub sustain_gap_cap: Option<Duration>,
    /// Goggles 2: re-arm once video has been silent this long...
    pub g2_silence: Duration,
    /// ...but never sooner than this after the previous arming burst.
    pub g2_rearm_settle: Duration,
    pub poll: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            g3_probe: Duration::from_secs(8),
            g3_probe_max: Duration::from_secs(120),
            g2_probe: Duration::from_secs(12),
            g3_period: Duration::from_secs(3),
            arm_gap_cap: g2::ARM_GAP_CAP,
            sustain_gap_cap: None,
            g2_silence: Duration::from_secs(6),
            g2_rearm_settle: g2::REARM_SETTLE,
            poll: Duration::from_millis(100),
        }
    }
}

struct Driver<'a> {
    cfg: &'a Config,
    shared: &'a Shared,
    t: &'a Timing,
    tx: &'a mut dyn FnMut(&[u8]) -> io::Result<()>,
    report: &'a dyn Fn(Value),
    seq: u16,
    locked: bool,
    /// These goggles showed video with exactly one protocol before: that one
    /// is used and never given up on.
    pinned: bool,
    started: Instant,
    id: Option<String>,
    /// Video byte count and time of its last increase.
    seen: u64,
    last_video: Option<Instant>,
}

/// Drive the session until a USB write fails (the goggles went away).
pub fn drive(
    cfg: &Config,
    shared: &Shared,
    t: &Timing,
    tx: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    report: &dyn Fn(Value),
) -> io::Result<()> {
    let forced = Goggles::from_id(&cfg.mode);
    let id = identification(&cfg.aoa);
    let remembered = match (forced, &cfg.memory, &id) {
        (None, Some(path), Some(id)) => Memory::load(path).preferred(id),
        _ => None,
    };
    if forced.is_none() {
        eprintln!(
            "Goggles identification {}: {}",
            id.as_deref().unwrap_or("none"),
            remembered.map_or("unknown or ambiguous; Goggles 3 first", |g| g.label())
        );
    }
    let pinned = forced.is_none() && remembered.is_some();
    if pinned {
        eprintln!("Goggles protocol stays with the remembered one; no probing of the other");
    }
    let mut current = forced.or(remembered).unwrap_or(Goggles::G3);
    let mut d = Driver {
        cfg,
        shared,
        t,
        tx,
        report,
        seq: 0x100,
        locked: false,
        pinned,
        started: Instant::now(),
        id,
        seen: shared.video.load(Ordering::Relaxed),
        last_video: None,
    };
    let mut g3_probe = t.g3_probe;
    loop {
        d.shared.profile.store(current as u8, Ordering::Relaxed);
        d.announce(current);
        let probe = match current {
            Goggles::G3 => g3_probe,
            Goggles::G2 => t.g2_probe,
        };
        // Forced and remembered protocols never time out.
        let deadline = (forced.is_none() && !pinned).then(|| Instant::now() + probe);
        match current {
            Goggles::G3 => d.run_g3(deadline)?,
            Goggles::G2 => d.run_g2(deadline)?,
        }
        eprintln!(
            "No video with the {} protocol after {}s; trying {}",
            current.label(),
            probe.as_secs(),
            current.other().label()
        );
        if current == Goggles::G3 {
            g3_probe = (g3_probe * 2).min(t.g3_probe_max);
        }
        current = current.other();
    }
}

impl Driver<'_> {
    fn announce(&self, g: Goggles) {
        let state = if self.locked {
            "video"
        } else if Goggles::from_id(&self.cfg.mode).is_some() {
            "forced"
        } else if self.pinned {
            "remembered"
        } else {
            "probing"
        };
        let after = if self.locked {
            format!(
                ", first video {:.1}s after the session started",
                self.started.elapsed().as_secs_f64()
            )
        } else {
            String::new()
        };
        eprintln!("Goggles protocol: {} ({state}{after})", g.label());
        (self.report)(json!({"goggles_protocol":g.id(),"goggles_protocol_state":state}));
    }

    fn send(&mut self, packet: &[u8]) -> io::Result<()> {
        (self.tx)(packet)
    }

    /// Track video progress; lock the protocol on the first video bytes.
    fn observe(&mut self, g: Goggles) {
        let now = self.shared.video.load(Ordering::Relaxed);
        if now > self.seen {
            self.seen = now;
            self.last_video = Some(Instant::now());
            if !self.locked {
                self.locked = true;
                if Goggles::from_id(&self.cfg.mode).is_none()
                    && let (Some(path), Some(id)) = (&self.cfg.memory, &self.id)
                {
                    let mut memory = Memory::load(path);
                    // Write only when something new was learned: SD card wear.
                    if memory.learn(id, g)
                        && let Err(e) = path
                            .parent()
                            .map_or(Ok(()), std::fs::create_dir_all)
                            .map_err(anyhow::Error::from)
                            .and_then(|()| {
                                crate::settings::atomic_write(
                                    path,
                                    &serde_json::to_vec_pretty(&memory)?,
                                )
                            })
                    {
                        eprintln!("Cannot remember goggles protocol: {e:#}");
                    }
                }
                self.announce(g);
            }
        }
    }

    /// True once an unlocked probe has run out of time.
    fn give_up(&self, deadline: Option<Instant>) -> bool {
        !self.locked && deadline.is_some_and(|d| Instant::now() >= d)
    }

    /// Sleep in poll slices, observing video. Returns true to give up.
    fn wait(&mut self, g: Goggles, total: Duration, deadline: Option<Instant>) -> bool {
        let end = Instant::now() + total;
        loop {
            self.observe(g);
            if self.give_up(deadline) {
                return true;
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            thread::sleep(left.min(self.t.poll));
        }
    }

    fn run_g3(&mut self, deadline: Option<Instant>) -> io::Result<()> {
        loop {
            for packet in protocol::registration(&mut self.seq, self.cfg.g3_port) {
                self.send(&packet)?;
            }
            if self.cfg.sync_time
                && let Some(clock) = clock::now()
            {
                let packet = protocol::time_sync(&mut self.seq, self.cfg.g3_port, &clock);
                self.send(&packet)?;
            }
            if self.wait(Goggles::G3, self.t.g3_period, deadline) {
                return Ok(());
            }
        }
    }

    fn arm_g2(&mut self, deadline: Option<Instant>) -> io::Result<bool> {
        let clock = if self.cfg.sync_time {
            clock::now()
        } else {
            None
        };
        let steps = g2::arm();
        eprintln!("Goggles 2: arming ({} requests)", steps.len());
        for (i, step) in steps.iter().enumerate() {
            let gap = g2::gap(steps, i, Some(self.t.arm_gap_cap));
            if !gap.is_zero() && self.wait(Goggles::G2, gap, deadline) {
                return Ok(true);
            }
            let packet = g2::packet(step, &mut self.seq, clock.as_ref());
            self.send(&packet)?;
        }
        Ok(false)
    }

    fn run_g2(&mut self, deadline: Option<Instant>) -> io::Result<()> {
        if self.arm_g2(deadline)? {
            return Ok(());
        }
        let mut armed = Instant::now();
        loop {
            let steps = g2::sustain();
            for (i, step) in steps.iter().enumerate() {
                let gap = g2::gap(steps, i, self.t.sustain_gap_cap);
                if self.wait(Goggles::G2, gap, deadline) {
                    return Ok(());
                }
                let packet = g2::packet(step, &mut self.seq, None);
                self.send(&packet)?;
            }
            // Keepalives alone do not resume video after the air unit was
            // power-cycled, or when it was switched on after the goggles were
            // plugged in. Re-arm then, but never in quick succession.
            let silent = self
                .last_video
                .map_or(armed.elapsed(), |t| t.elapsed())
                .min(armed.elapsed());
            let quiet_enough = if self.last_video.is_some() {
                silent >= self.t.g2_silence
            } else {
                // Never saw video: only a forced or locked profile keeps
                // re-arming; an auto probe moves on to the other protocol.
                deadline.is_none()
            };
            if quiet_enough && armed.elapsed() >= self.t.g2_rearm_settle {
                eprintln!("Goggles 2: no video for {}s; re-arming", silent.as_secs());
                if self.arm_g2(deadline)? {
                    return Ok(());
                }
                armed = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn fast() -> Timing {
        Timing {
            g3_probe: Duration::from_millis(300),
            g3_probe_max: Duration::from_millis(1000),
            g2_probe: Duration::from_millis(400),
            g3_period: Duration::from_millis(50),
            arm_gap_cap: Duration::ZERO,
            sustain_gap_cap: Some(Duration::from_millis(2)),
            g2_silence: Duration::from_millis(150),
            g2_rearm_settle: Duration::from_millis(300),
            poll: Duration::from_millis(5),
        }
    }

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Model {
        G3,
        G2,
    }

    /// What the simulated goggles saw: (port, cmd_set, cmd_id) per request.
    type Log = Arc<Mutex<Vec<(Instant, u16, u8, u8)>>>;

    /// Simulated goggles. Video flows once the air unit is on (after
    /// `drone_on`) and the goggles got their own start request, and stops
    /// while the air unit is off. Goggles 3 start on the mobile registration
    /// (0/0x99) on the G3 port; Goggles 2 on the camera live-view
    /// subscription (2/0xeb) on 0x7530 after an arming burst (0/0xdd).
    fn simulate(
        model: Model,
        mode: &str,
        memory: Option<PathBuf>,
        aoa: &[&str],
        run_for: Duration,
        drone_on: Duration,
        drone_off: Option<(Duration, Duration)>,
    ) -> (Log, Vec<Value>) {
        let shared = Arc::new(Shared::default());
        let log: Log = Arc::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let start = Instant::now();
        let streaming = Arc::new(Mutex::new(false));
        let armed = Arc::new(Mutex::new(false));
        let air_on = move || {
            let t = start.elapsed();
            t >= drone_on && !drone_off.is_some_and(|(a, b)| t >= a && t < b)
        };
        let feeder = {
            let shared = shared.clone();
            let streaming = streaming.clone();
            thread::spawn(move || {
                while start.elapsed() < run_for {
                    if *streaming.lock().unwrap() && air_on() {
                        shared.video.fetch_add(1000, Ordering::Relaxed);
                    } else {
                        // Air unit power-cycled: goggles stop until re-requested.
                        *streaming.lock().unwrap() = false;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
        };
        let cfg = Config {
            mode: mode.into(),
            g3_port: 0x5749,
            sync_time: false,
            memory,
            aoa: aoa.iter().map(|s| s.to_string()).collect(),
        };
        let t = fast();
        let l = log.clone();
        let mut tx = |p: &[u8]| -> io::Result<()> {
            if start.elapsed() >= run_for {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let port = u16::from_le_bytes([p[2], p[3]]);
            let duml = &p[8..];
            assert!(protocol::valid(duml));
            let (set, id) = (duml[9], duml[10]);
            l.lock().unwrap().push((Instant::now(), port, set, id));
            let mut s = streaming.lock().unwrap();
            match model {
                Model::G3 if port == 0x5749 && (set, id) == (0, 0x99) && air_on() => *s = true,
                Model::G2 if port == 0x7530 && (set, id) == (0, 0xdd) => {
                    *armed.lock().unwrap() = true
                }
                Model::G2
                    if port == 0x7530
                        && (set, id) == (2, 0xeb)
                        && *armed.lock().unwrap()
                        && air_on() =>
                {
                    *s = true
                }
                _ => {}
            }
            Ok(())
        };
        let ev = events.clone();
        let report = move |v: Value| ev.lock().unwrap().push(v);
        let result = drive(&cfg, &shared, &t, &mut tx, &report);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        feeder.join().unwrap();
        let events = events.lock().unwrap().clone();
        (log, events)
    }

    fn final_state(events: &[Value]) -> (String, String) {
        let last = events.last().unwrap();
        (
            last["goggles_protocol"].as_str().unwrap().into(),
            last["goggles_protocol_state"].as_str().unwrap().into(),
        )
    }
    fn on_port(log: &Log, port: u16) -> usize {
        log.lock().unwrap().iter().filter(|e| e.1 == port).count()
    }
    fn arming_bursts(log: &Log) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| e.1 == 0x7530 && (e.2, e.3) == (0, 0xdd))
            .count()
            / 4 // each burst contains the 0/0xdd activation four times
    }
    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ungoggled-{name}-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }
    const S: fn(u64) -> Duration = Duration::from_millis;

    /// AOA strings the Goggles 2 reference recorded; Goggles 3 strings are
    /// not documented, so the tests cover both distinct and identical ones.
    const G2_AOA: &[&str] = &[
        "DJI",
        "com.dji.logiclink",
        "DJI glass",
        "v0.0.0.0",
        "www.dji.com",
        "000000000000000",
    ];
    const OTHER_AOA: &[&str] = &[
        "DJI",
        "com.dji.logiclink",
        "DJI goggles 3",
        "v1.0.0.0",
        "www.dji.com",
        "1581F6Z9C23A",
    ];
    fn learned(mem: &std::path::Path, aoa: &[&str]) -> Vec<String> {
        let id = identification(&aoa.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        Memory::load(mem)
            .goggles
            .into_iter()
            .find(|e| e.id == id)
            .map(|e| e.protocols)
            .unwrap_or_default()
    }
    fn first_port(log: &Log) -> u16 {
        log.lock().unwrap()[0].1
    }

    #[test]
    fn goggles3_locks_immediately_and_never_sends_goggles2_requests() {
        let mem = temp("g3");
        let (log, events) = simulate(
            Model::G3,
            "auto",
            Some(mem.clone()),
            OTHER_AOA,
            S(1500),
            S(0),
            None,
        );
        assert_eq!(final_state(&events), ("goggles3".into(), "video".into()));
        assert_eq!(on_port(&log, 0x7530), 0);
        assert_eq!(learned(&mem, OTHER_AOA), ["goggles3"]);
    }

    #[test]
    fn goggles2_found_after_goggles3_probe_and_learned() {
        let mem = temp("g2");
        let (log, events) = simulate(
            Model::G2,
            "auto",
            Some(mem.clone()),
            G2_AOA,
            S(2500),
            S(0),
            None,
        );
        assert_eq!(final_state(&events), ("goggles2".into(), "video".into()));
        assert_eq!(learned(&mem, G2_AOA), ["goggles2"]);
        // Once locked, no further Goggles 3 registrations are sent.
        let first_g2 = log
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.1 == 0x7530)
            .unwrap()
            .0;
        let late_g3 = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.1 == 0x5749 && e.0 > first_g2)
            .count();
        assert_eq!(late_g3, 0);
        assert_eq!(
            arming_bursts(&log),
            1,
            "streaming goggles must not be re-armed"
        );

        // The same goggles start with Goggles 2 straight away next time.
        let (log, events) = simulate(
            Model::G2,
            "auto",
            Some(mem.clone()),
            G2_AOA,
            S(800),
            S(0),
            None,
        );
        assert_eq!(first_port(&log), 0x7530);
        assert_eq!(on_port(&log, 0x5749), 0);
        assert_eq!(final_state(&events).1, "video");
    }

    #[test]
    fn swapping_to_goggles3_with_different_identification_starts_with_goggles3() {
        let mem = temp("swap-distinct");
        simulate(
            Model::G2,
            "auto",
            Some(mem.clone()),
            G2_AOA,
            S(2500),
            S(0),
            None,
        );
        assert_eq!(learned(&mem, G2_AOA), ["goggles2"]);
        // A Goggles 3 pilot plugs in after the Goggles 2 flight.
        let (log, events) = simulate(
            Model::G3,
            "auto",
            Some(mem.clone()),
            OTHER_AOA,
            S(800),
            S(0),
            None,
        );
        assert_eq!(first_port(&log), 0x5749);
        assert_eq!(on_port(&log, 0x7530), 0, "no Goggles 2 detour");
        assert_eq!(final_state(&events), ("goggles3".into(), "video".into()));
        // The Goggles 2 still start directly afterwards.
        let (log, _) = simulate(
            Model::G2,
            "auto",
            Some(mem.clone()),
            G2_AOA,
            S(800),
            S(0),
            None,
        );
        assert_eq!(first_port(&log), 0x7530);
    }

    #[test]
    fn an_old_memory_file_with_both_protocols_prefers_the_quick_goggles3_probe() {
        let mem = temp("swap-same");
        // Written by an older release that could learn both for one pair of strings.
        let id = identification(&G2_AOA.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        let mut m = Memory::default();
        m.learn(&id, Goggles::G2);
        m.learn(&id, Goggles::G3);
        std::fs::write(&mem, serde_json::to_vec(&m).unwrap()).unwrap();
        // Both start with the short Goggles 3 probe and still work.
        let (log, events) = simulate(
            Model::G3,
            "auto",
            Some(mem.clone()),
            G2_AOA,
            S(800),
            S(0),
            None,
        );
        assert_eq!(on_port(&log, 0x7530), 0);
        assert_eq!(final_state(&events), ("goggles3".into(), "video".into()));
        let (log, events) = simulate(
            Model::G2,
            "auto",
            Some(mem.clone()),
            G2_AOA,
            S(2500),
            S(0),
            None,
        );
        assert_eq!(first_port(&log), 0x5749);
        assert_eq!(final_state(&events), ("goggles2".into(), "video".into()));
    }

    #[test]
    fn without_identification_nothing_is_stored() {
        let mem = temp("noid");
        let (_, events) = simulate(
            Model::G2,
            "auto",
            Some(mem.clone()),
            &[],
            S(2500),
            S(0),
            None,
        );
        assert_eq!(final_state(&events), ("goggles2".into(), "video".into()));
        assert!(!mem.exists());
    }

    #[test]
    fn memory_is_bounded_and_ignores_corrupt_files() {
        let mem = temp("bounded");
        std::fs::write(&mem, "not json").unwrap();
        let (log, _) = simulate(
            Model::G3,
            "auto",
            Some(mem.clone()),
            OTHER_AOA,
            S(600),
            S(0),
            None,
        );
        assert_eq!(first_port(&log), 0x5749);
        let mut m = Memory::default();
        for i in 0..40 {
            m.learn(&format!("{i}"), Goggles::G3);
        }
        assert_eq!(m.goggles.len(), MEMORY_LIMIT);
        assert_eq!(m.goggles[0].id, "8");
    }

    #[test]
    fn late_drone_power_on_is_found_by_alternating() {
        for model in [Model::G3, Model::G2] {
            let (_, events) = simulate(model, "auto", None, G2_AOA, S(4000), S(1200), None);
            let want = if model == Model::G3 {
                "goggles3"
            } else {
                "goggles2"
            };
            assert_eq!(
                final_state(&events),
                (want.into(), "video".into()),
                "{model:?}"
            );
        }
    }

    #[test]
    fn goggles2_rearms_after_air_unit_power_cycle() {
        let (log, events) = simulate(
            Model::G2,
            "goggles2",
            None,
            G2_AOA,
            S(3000),
            S(0),
            Some((S(800), S(1300))),
        );
        assert_eq!(final_state(&events).0, "goggles2");
        assert_eq!(on_port(&log, 0x5749), 0, "forced profile never probes");
        let bursts = arming_bursts(&log);
        assert!((2..=6).contains(&bursts), "{bursts} arming bursts");
    }

    #[test]
    fn forced_goggles3_never_switches() {
        let (log, events) = simulate(Model::G2, "goggles3", None, G2_AOA, S(1200), S(0), None);
        assert_eq!(on_port(&log, 0x7530), 0);
        assert_eq!(final_state(&events), ("goggles3".into(), "forced".into()));
    }

    /// Writes a memory file saying these goggles only ever worked with `g`.
    fn remember(mem: &std::path::Path, aoa: &[&str], g: Goggles) {
        let id = identification(&aoa.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        let mut m = Memory::default();
        m.learn(&id, g);
        std::fs::write(mem, serde_json::to_vec(&m).unwrap()).unwrap();
    }

    #[test]
    fn remembered_goggles3_never_get_the_goggles2_burst_while_the_drone_is_off() {
        let mem = temp("pinned-g3");
        remember(&mem, OTHER_AOA, Goggles::G3);
        // The drone is switched on long after any probe would have expired.
        let (log, events) = simulate(
            Model::G3,
            "auto",
            Some(mem),
            OTHER_AOA,
            S(3500),
            S(2500),
            None,
        );
        assert_eq!(on_port(&log, 0x7530), 0, "no Goggles 2 request at all");
        assert_eq!(final_state(&events), ("goggles3".into(), "video".into()));
        assert!(
            events
                .iter()
                .any(|e| e["goggles_protocol_state"] == "remembered"),
            "reported as remembered while waiting"
        );
    }

    #[test]
    fn remembered_goggles2_stay_with_goggles2() {
        let mem = temp("pinned-g2");
        remember(&mem, G2_AOA, Goggles::G2);
        let (log, events) = simulate(Model::G2, "auto", Some(mem), G2_AOA, S(3500), S(2500), None);
        assert_eq!(on_port(&log, 0x5749), 0, "no Goggles 3 registration");
        assert_eq!(final_state(&events), ("goggles2".into(), "video".into()));
    }

    #[test]
    fn unknown_goggles_without_a_drone_get_fewer_and_fewer_bursts() {
        let (log, _) = simulate(Model::G3, "auto", None, G2_AOA, S(5000), S(60_000), None);
        let bursts = arming_bursts(&log);
        // 300 ms Goggles 3, 400 ms Goggles 2, then 600, 1000, 1000... of
        // Goggles 3 between the bursts: 4 in 5 s instead of one every 0.7 s.
        assert!((3..=5).contains(&bursts), "{bursts} arming bursts");
    }
}
