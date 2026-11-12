use anyhow::{Result, bail, ensure};
use std::{
    fs,
    os::unix::fs::symlink,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const MIN_UNIX_MS: u64 = 1_577_836_800_000; // 2020-01-01
const MAX_UNIX_MS: u64 = 4_102_444_800_000; // 2100-01-01

pub struct Wall {
    pub unix_ms: u64,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub tz: String,
}

impl Wall {
    pub fn display(&self) -> String {
        format!(
            "{}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

pub fn now() -> Option<Wall> {
    let unix_ms = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis(),
    )
    .ok()?;
    broken_down(unix_ms)
}

fn broken_down(unix_ms: u64) -> Option<Wall> {
    #[allow(deprecated)]
    let t = libc::time_t::try_from(unix_ms / 1000).ok()?;
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(Wall {
        unix_ms,
        year: u16::try_from(tm.tm_year.checked_add(1900)?).ok()?,
        month: u8::try_from(tm.tm_mon.checked_add(1)?).ok()?,
        day: u8::try_from(tm.tm_mday).ok()?,
        hour: u8::try_from(tm.tm_hour).ok()?,
        minute: u8::try_from(tm.tm_min).ok()?,
        second: u8::try_from(tm.tm_sec).ok()?,
        tz: tz_id(),
    })
}

fn tz_id() -> String {
    if let Ok(target) = fs::read_link("/etc/localtime") {
        let target = target.to_string_lossy();
        if let Some(rest) = target.split("zoneinfo/").nth(1)
            && valid_timezone(rest)
        {
            return rest.into();
        }
    }
    if let Ok(text) = fs::read_to_string("/etc/timezone") {
        let text = text.trim();
        if valid_timezone(text) {
            return text.into();
        }
    }
    "UTC".into()
}

/// True once the kernel clock is disciplined by NTP (e.g. on a Wi-Fi client
/// with Internet access); `adjtimex` reports TIME_ERROR while unsynchronized.
/// Created once the Pi's clock has been set from a browser or by NTP since
/// boot. It lives in `/run` (gone with the next reboot, as the clock's
/// trustworthiness is: the Pi has no battery clock) and outside the service's
/// own runtime directory, so a restart of the service keeps it. The goggles'
/// worker reads it before telling the air unit what time it is.
pub const SYNCED_MARKER: &str = "/run/ungoggled-clock-synced";
pub fn mark_synced() {
    let _ = std::fs::write(SYNCED_MARKER, b"");
}
pub fn marked_synced() -> bool {
    Path::new(SYNCED_MARKER).exists()
}
pub fn ntp_synchronized() -> bool {
    let mut tx = unsafe { std::mem::zeroed::<libc::timex>() };
    let state = unsafe { libc::adjtimex(&mut tx) };
    state >= 0 && state != libc::TIME_ERROR
}

pub fn valid_timezone(tz: &str) -> bool {
    if tz.is_empty() || tz.len() > 64 || tz.contains("..") || Path::new(tz).is_absolute() {
        return false;
    }
    if !tz
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'+' | b'-'))
    {
        return false;
    }
    let root = Path::new("/usr/share/zoneinfo");
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    let path = root.join(tz);
    path.is_file() && fs::canonicalize(&path).is_ok_and(|p| p.starts_with(&root))
}

pub fn parse(unix_ms: u64, timezone: Option<&str>) -> Result<(u64, Option<String>)> {
    ensure!(
        (MIN_UNIX_MS..MAX_UNIX_MS).contains(&unix_ms),
        "Time is outside 2020–2100"
    );
    match timezone {
        None | Some("") => Ok((unix_ms, None)),
        Some(tz) => {
            ensure!(valid_timezone(tz), "Unknown timezone");
            Ok((unix_ms, Some(tz.into())))
        }
    }
}

pub fn apply(unix_ms: u64, timezone: Option<&str>) -> Result<()> {
    let (unix_ms, tz) = parse(unix_ms, timezone)?;
    let _ = Command::new("timedatectl")
        .args(["set-ntp", "false"])
        .status();
    if let Some(tz) = tz {
        set_timezone(&tz)?;
    }
    set_realtime(unix_ms)?;
    remember_time();
    let _ = Command::new("fake-hwclock").arg("save").status();
    Ok(())
}

/// systemd sets the clock to the modification time of this file early in every
/// boot, if the clock is behind it. The Pi has no battery clock and no NTP
/// (setting the time from a browser also switches `systemd-timesyncd` off, the
/// one thing that would keep the file current), so without this every boot
/// starts at the day the image was built, and the log, the recordings and the
/// file dates carry that date until somebody opens the web interface.
const CLOCK_FILE: &str = "/var/lib/systemd/timesync/clock";
/// Remember "now" for the next boot (see `CLOCK_FILE`). Only call it while the
/// clock is right.
pub fn remember_time() {
    remember_time_at(Path::new(CLOCK_FILE));
}
fn remember_time_at(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Writing (even nothing) sets the modification time to now.
    let _ = std::fs::write(path, b"");
}

fn set_timezone(tz: &str) -> Result<()> {
    let timedatectl = Command::new("timedatectl")
        .args(["set-timezone", tz])
        .status();
    if !timedatectl.map(|s| s.success()).unwrap_or(false) {
        let dest = Path::new("/usr/share/zoneinfo").join(tz);
        let tmp = Path::new("/etc/localtime.set");
        let _ = fs::remove_file(tmp);
        symlink(&dest, tmp)?;
        fs::rename(tmp, "/etc/localtime")?;
    }
    let _ = fs::write("/etc/timezone", format!("{tz}\n"));
    Ok(())
}

fn set_realtime(unix_ms: u64) -> Result<()> {
    #[allow(deprecated)]
    let ts = libc::timespec {
        tv_sec: (unix_ms / 1000) as libc::time_t,
        tv_nsec: ((unix_ms % 1000) * 1_000_000) as libc::c_long,
    };
    if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } != 0 {
        bail!(
            "Could not set the system clock: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn remember_time_writes_the_current_time() {
        let dir = std::env::temp_dir().join(format!("ungoggled-clockfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("timesync/clock");
        // The directory does not exist yet (a fresh image): it is created.
        super::remember_time_at(&file);
        let first = std::fs::metadata(&file).unwrap().modified().unwrap();
        assert!(first.elapsed().unwrap() < std::time::Duration::from_secs(5));
        // An old file gets its date moved up.
        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * 56);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        super::remember_time_at(&file);
        assert!(
            std::fs::metadata(&file)
                .unwrap()
                .modified()
                .unwrap()
                .elapsed()
                .unwrap()
                < std::time::Duration::from_secs(5)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;
    #[test]
    fn rejects_out_of_range_and_unsafe_timezones() {
        assert!(parse(0, None).is_err());
        assert!(parse(MIN_UNIX_MS - 1, None).is_err());
        assert!(parse(MAX_UNIX_MS, None).is_err());
        assert!(parse(MIN_UNIX_MS, Some("/etc/passwd")).is_err());
        assert!(parse(MIN_UNIX_MS, Some("../etc/passwd")).is_err());
        assert!(parse(MIN_UNIX_MS, Some("Europe/Berlin\n")).is_err());
        assert!(parse(MIN_UNIX_MS, Some("")).is_ok());
        assert!(parse(1_700_000_000_000, None).is_ok());
    }
    #[test]
    fn accepts_installed_zoneinfo() {
        if Path::new("/usr/share/zoneinfo/UTC").is_file() {
            assert!(valid_timezone("UTC"));
            assert!(parse(1_700_000_000_000, Some("UTC")).is_ok());
        }
        if Path::new("/usr/share/zoneinfo/Europe/Berlin").is_file() {
            assert!(valid_timezone("Europe/Berlin"));
        }
        assert!(!valid_timezone(""));
        assert!(!valid_timezone(".."));
    }
    #[test]
    fn now_is_plausible() {
        let clock = now().expect("local clock");
        assert!((MIN_UNIX_MS..MAX_UNIX_MS).contains(&clock.unix_ms));
        assert!((1970..=2100).contains(&clock.year));
        assert!((1..=12).contains(&clock.month));
        assert!((1..=31).contains(&clock.day));
        assert!(clock.hour <= 23);
        assert!(clock.minute <= 59);
        assert!(clock.second <= 60);
        assert!(!clock.tz.is_empty());
        assert_eq!(clock.display().len(), 19);
    }
}
