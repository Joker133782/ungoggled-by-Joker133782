use crate::metrics::Metrics;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub const CAPACITY: usize = 1800;
pub struct History {
    started: Instant,
    /// Identifies this run of the service: `t` restarts from zero with every
    /// start, so a browser holding samples from an earlier run must not mix
    /// them with this one's (see `snapshot`).
    instance: u64,
    samples: VecDeque<Value>,
}
impl History {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            instance: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            samples: VecDeque::with_capacity(CAPACITY),
        }
    }
    pub fn push(&mut self, s: &Value, m: &Metrics) {
        if self.samples.len() == CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(json!({
            "t":self.started.elapsed().as_secs_f64(), "bitrate_mbps":s["bitrate_mbps"],
            "input_fps":s["input_fps"], "output_fps":s["output_fps"],
            "temperature_c":s["temperature_c"], "phase":s["phase"], "hdmi":s["hdmi"],
            "discarded_bytes":s["discarded_bytes"], "decoder_starts":s["decoder_starts"],
            "cpu_percent":m.cpu_percent, "ram_used_mib":m.ram_used_mib,
            "ram_total_mib":m.ram_total_mib, "disk_read_kib_s":m.disk_read_kib_s,
            "disk_write_kib_s":m.disk_write_kib_s, "wifi_rx_mbps":m.wifi_rx_mbps,
            "wifi_tx_mbps":m.wifi_tx_mbps, "lan_rx_mbps":m.lan_rx_mbps,
            "lan_tx_mbps":m.lan_tx_mbps
        }));
    }
    /// All retained samples, or with `since` only those taken after that
    /// `t`: the graphs fetch everything once, then just what's new every few
    /// seconds instead of the whole half hour again. A browser compares
    /// `instance` to notice a restart, after which its `since` would refer
    /// to the previous run.
    pub fn snapshot(&self, since: Option<f64>) -> Value {
        let first = since.map_or(0, |since| {
            self.samples
                .partition_point(|s| s["t"].as_f64().is_some_and(|t| t <= since))
        });
        let samples: Vec<&Value> = self.samples.range(first..).collect();
        json!({
            "duration_seconds": 1800,
            "sample_interval_seconds": 1,
            "instance": self.instance,
            "now": self.started.elapsed().as_secs_f64(),
            "samples": samples,
        })
    }
}
pub fn temperature() -> Option<f64> {
    let n: f64 = std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (n.is_finite() && (0.0..150_000.0).contains(&n)).then_some(n / 1000.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_bounded_and_keeps_latest() {
        let mut h = History::new();
        for n in 0..2000 {
            h.push(&json!({"bitrate_mbps":n}), &Metrics::default());
        }
        assert_eq!(h.samples.len(), 1800);
        assert_eq!(h.samples.front().unwrap()["bitrate_mbps"], 200);
        assert_eq!(h.samples.back().unwrap()["bitrate_mbps"], 1999);
    }
    #[test]
    fn since_returns_only_newer_samples() {
        let mut h = History::new();
        for n in 0..5 {
            h.push(&json!({"bitrate_mbps":n}), &Metrics::default());
        }
        let all = h.snapshot(None);
        let samples = all["samples"].as_array().unwrap();
        assert_eq!(samples.len(), 5);
        let third = samples[2]["t"].as_f64().unwrap();
        let newer = h.snapshot(Some(third));
        let newer: Vec<_> = newer["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["bitrate_mbps"].as_i64().unwrap())
            .collect();
        assert_eq!(newer, [3, 4]);
        let now = all["now"].as_f64().unwrap();
        assert!(
            h.snapshot(Some(now))["samples"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            h.snapshot(Some(-1.0))["samples"].as_array().unwrap().len(),
            5
        );
        assert_eq!(all["instance"], h.snapshot(Some(third))["instance"]);
    }
}
