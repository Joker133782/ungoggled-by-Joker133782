//! System resource sampling for the web UI's history graphs: CPU load, RAM
//! use, SD card and network throughput. Linux-only (`/proc`, `/sys`), no
//! external crate — these are all plain text files on any Linux box.
use std::{fs, time::Instant};

#[derive(Default, Clone, Copy, serde::Serialize)]
pub struct Metrics {
    pub cpu_percent: f64,
    pub ram_used_mib: f64,
    pub ram_total_mib: f64,
    pub disk_read_kib_s: f64,
    pub disk_write_kib_s: f64,
    pub wifi_rx_mbps: f64,
    pub wifi_tx_mbps: f64,
    pub lan_rx_mbps: f64,
    pub lan_tx_mbps: f64,
}

#[derive(Clone, Copy, Default)]
struct Counters {
    cpu_total: u64,
    cpu_idle: u64,
    disk_read_sectors: u64,
    disk_write_sectors: u64,
    wifi_rx: u64,
    wifi_tx: u64,
    lan_rx: u64,
    lan_tx: u64,
}

/// Call `sample()` once a second (matches the history sampler in `main.rs`):
/// the throughput fields are rates computed from consecutive reads, so the
/// first call after a restart always reports zero for them.
#[derive(Default)]
pub struct Sampler {
    prev: Option<(Instant, Counters)>,
}
impl Sampler {
    pub fn sample(&mut self) -> Metrics {
        let now = Instant::now();
        let cur = read_counters();
        let (ram_used_mib, ram_total_mib) = read_mem();
        let mut m = Metrics {
            ram_used_mib,
            ram_total_mib,
            ..Default::default()
        };
        if let Some((prev_at, prev)) = self.prev {
            let dt = now.duration_since(prev_at).as_secs_f64().max(0.001);
            let cpu_total_d = cur.cpu_total.saturating_sub(prev.cpu_total) as f64;
            let cpu_idle_d = cur.cpu_idle.saturating_sub(prev.cpu_idle) as f64;
            m.cpu_percent = if cpu_total_d > 0.0 {
                ((cpu_total_d - cpu_idle_d) / cpu_total_d * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            // diskstats sectors are always 512 bytes, regardless of the
            // device's real block size.
            m.disk_read_kib_s =
                sub(cur.disk_read_sectors, prev.disk_read_sectors) as f64 * 0.5 / dt;
            m.disk_write_kib_s =
                sub(cur.disk_write_sectors, prev.disk_write_sectors) as f64 * 0.5 / dt;
            m.wifi_rx_mbps = mbps(sub(cur.wifi_rx, prev.wifi_rx), dt);
            m.wifi_tx_mbps = mbps(sub(cur.wifi_tx, prev.wifi_tx), dt);
            m.lan_rx_mbps = mbps(sub(cur.lan_rx, prev.lan_rx), dt);
            m.lan_tx_mbps = mbps(sub(cur.lan_tx, prev.lan_tx), dt);
        }
        self.prev = Some((now, cur));
        m
    }
}
fn sub(a: u64, b: u64) -> u64 {
    a.saturating_sub(b)
}
fn mbps(bytes: u64, dt: f64) -> f64 {
    bytes as f64 * 8.0 / 1_000_000.0 / dt
}

fn read_counters() -> Counters {
    let (cpu_total, cpu_idle) = read_cpu();
    let (disk_read_sectors, disk_write_sectors) = read_disk();
    let (wifi_rx, wifi_tx) = read_net("wlan0");
    let (lan_rx, lan_tx) = read_net("eth0");
    Counters {
        cpu_total,
        cpu_idle,
        disk_read_sectors,
        disk_write_sectors,
        wifi_rx,
        wifi_tx,
        lan_rx,
        lan_tx,
    }
}
/// First line of `/proc/stat`: `cpu  user nice system idle iowait irq softirq steal`.
fn read_cpu() -> (u64, u64) {
    let Ok(s) = fs::read_to_string("/proc/stat") else {
        return (0, 0);
    };
    let Some(line) = s.lines().next() else {
        return (0, 0);
    };
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    let total = fields.iter().sum();
    // idle (3) + iowait (4): both count as "not busy" for a load percentage.
    let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
    (total, idle)
}
/// `MemTotal`/`MemAvailable` from `/proc/meminfo`, in MiB. `MemAvailable`
/// (not `MemFree`) accounts for reclaimable cache, so it tracks what's
/// actually usable rather than alarming on normal page-cache growth.
fn read_mem() -> (f64, f64) {
    let Ok(s) = fs::read_to_string("/proc/meminfo") else {
        return (0.0, 0.0);
    };
    let kib = |key: &str| {
        s.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let total = kib("MemTotal:");
    let available = kib("MemAvailable:");
    (((total - available).max(0.0)) / 1024.0, total / 1024.0)
}
/// Sectors read/written for the SD card (`mmcblk0`) from `/proc/diskstats`:
/// `major minor name reads_completed reads_merged sectors_read ms_reading
/// writes_completed writes_merged sectors_written ...` — fields 3/6/10
/// (1-indexed), i.e. [2]/[5]/[9] after splitting.
fn read_disk() -> (u64, u64) {
    let Ok(s) = fs::read_to_string("/proc/diskstats") else {
        return (0, 0);
    };
    let Some(line) = s.lines().find(|l| {
        l.split_whitespace()
            .nth(2)
            .is_some_and(|name| name == "mmcblk0")
    }) else {
        return (0, 0);
    };
    let fields: Vec<&str> = line.split_whitespace().collect();
    let get = |i: usize| fields.get(i).and_then(|f| f.parse().ok()).unwrap_or(0);
    (get(5), get(9))
}
fn read_net(iface: &str) -> (u64, u64) {
    let read = |stat: &str| {
        fs::read_to_string(format!("/sys/class/net/{iface}/statistics/{stat}"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    (read("rx_bytes"), read("tx_bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_has_no_rates_yet() {
        let mut s = Sampler::default();
        let m = s.sample();
        assert_eq!(m.cpu_percent, 0.0);
        assert_eq!(m.wifi_rx_mbps, 0.0);
        // RAM is a point-in-time reading, not a rate, so it's never zero on
        // a real Linux box even on the first sample.
        assert!(m.ram_total_mib > 0.0);
    }

    #[test]
    fn second_sample_can_report_a_rate() {
        let mut s = Sampler::default();
        let _ = s.sample();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let m = s.sample();
        assert!(m.ram_total_mib > 0.0);
        assert!(m.ram_used_mib <= m.ram_total_mib);
    }

    #[test]
    fn mbps_converts_bytes_per_second_to_megabits() {
        assert!((mbps(125_000, 1.0) - 1.0).abs() < 1e-9);
    }
}
