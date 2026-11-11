//! Password gate for the control interface. Deliberately its own file
//! (`auth.json`), not a field on `settings::Settings`: that struct is
//! replaced wholesale on every settings save, so a password hash living
//! there would be one accidental unrelated save away from being wiped.
//!
//! Hashing is a from-scratch iterated SHA-256 (salted, 100k rounds) rather
//! than a real KDF crate (PBKDF2/Argon2): `sha2` is already a dependency and
//! pure Rust, and this guards a local Wi-Fi device against casual access,
//! not a stored secret worth a dedicated crate to defend at scale.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::Read,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const ROUNDS: u32 = 100_000;
const SALT_LEN: usize = 16;
const TOKEN_LEN: usize = 32;
/// A terminal (once built) turns a guessed password into a root shell, not
/// just a changed setting — unlimited guesses stopped being an acceptable
/// risk at that point.
const MAX_ATTEMPTS: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(5 * 60);
/// The per-address lockout is no use against someone who simply changes
/// their address, so wrong passwords from *all* addresses are counted too:
/// past `GLOBAL_FREE` of them within `GLOBAL_WINDOW`, every further check is
/// slowed down (not refused, so the owner can still get in), doubling up to
/// `GLOBAL_MAX_DELAY`.
const GLOBAL_FREE: u32 = 20;
const GLOBAL_WINDOW: Duration = Duration::from_secs(10 * 60);
const GLOBAL_MAX_DELAY: Duration = Duration::from_secs(30);
/// Shortest control password the server accepts (the web form says the same).
pub const MIN_PASSWORD_CHARS: usize = 8;

#[derive(Default)]
struct Attempts {
    failures: u32,
    last_failure: Option<Instant>,
    locked_until: Option<Instant>,
}

/// How long a check is held up after `recent` wrong passwords from anywhere.
fn global_delay(recent: u32) -> Duration {
    match recent.saturating_sub(GLOBAL_FREE) {
        0 => Duration::ZERO,
        excess => Duration::from_secs(1u64 << excess.min(5)).min(GLOBAL_MAX_DELAY),
    }
}

/// `Err(reason)` when the server will not take `password` as the control
/// password. It also becomes the Pi's root/SSH password, which is handed to
/// `chpasswd` as one `user:password` line, so line breaks and other control
/// characters cannot be allowed.
pub fn check_new_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        Err("Use at least 8 characters")
    } else if password.chars().any(char::is_control) {
        Err("The password must not contain line breaks or control characters")
    } else {
        Ok(())
    }
}

fn random_bytes(n: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .context("/dev/urandom")?;
    Ok(buf)
}

fn hash(password: &str, salt: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut out = Sha256::digest([salt, password.as_bytes()].concat()).to_vec();
    for _ in 1..ROUNDS {
        out = Sha256::digest(&out).to_vec();
    }
    out
}

/// Same-length, no-early-exit comparison: avoids leaking a match length
/// through response timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
/// A fresh random token (hex), the same kind as a session token — also used
/// for the terminal's single-use step-up tickets (`terminal.rs`).
pub fn random_token() -> Result<String> {
    Ok(to_hex(&random_bytes(TOKEN_LEN)?))
}
/// Constant-time comparison of two secret tokens.
pub fn tokens_equal(a: &str, b: &str) -> bool {
    ct_eq(a.as_bytes(), b.as_bytes())
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn from_hex(s: &str) -> Option<Vec<u8>> {
    s.len()
        .is_multiple_of(2)
        .then(|| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        })
        .flatten()
}

#[derive(Serialize, Deserialize, Clone)]
struct Credentials {
    salt: String,
    hash: String,
    /// Whether the Pi's root/SSH password has been set to this password yet
    /// (see `system_password`). Absent in files from before 0.3.52, which
    /// are brought in line at the next successful login.
    #[serde(default)]
    system_synced: bool,
}

/// Outcome of `Store::check`.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Right,
    Wrong,
    /// Refused without looking at the password; try again after this long.
    LockedOut(Duration),
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    creds: Arc<Mutex<Option<Credentials>>>,
    sessions: Arc<Mutex<HashSet<String>>>,
    attempts: Arc<Mutex<HashMap<IpAddr, Attempts>>>,
    /// Times of recent wrong passwords from any address (`GLOBAL_WINDOW`).
    recent_failures: Arc<Mutex<VecDeque<Instant>>>,
    /// Held for the whole of one `check`; see there.
    checking: Arc<Mutex<()>>,
    /// `auth.json` exists but cannot be read. Logins are refused (instead of
    /// the service failing to start, or the first-run setup being offered to
    /// whoever is on the Wi-Fi) until the owner resets the password.
    damaged: Arc<AtomicBool>,
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("auth.json");
        // A file from before 0.3.52 is world-readable.
        crate::settings::restrict_to_root(&path);
        let mut damaged = false;
        let creds = if path.exists() {
            match fs::read_to_string(&path)
                .with_context(|| format!("read {path:?}"))
                .and_then(|text| {
                    serde_json::from_str::<Credentials>(&text)
                        .with_context(|| format!("parse {path:?}"))
                }) {
                Ok(creds) => Some(creds),
                Err(e) => {
                    eprintln!("Control password file unusable, logins are refused: {e:#}");
                    damaged = true;
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            path,
            creds: Arc::new(Mutex::new(creds)),
            sessions: Arc::new(Mutex::new(HashSet::new())),
            attempts: Arc::new(Mutex::new(HashMap::new())),
            recent_failures: Arc::new(Mutex::new(VecDeque::new())),
            checking: Arc::new(Mutex::new(())),
            damaged: Arc::new(AtomicBool::new(damaged)),
        })
    }
    /// The password file is there but unusable; see the field.
    pub fn damaged(&self) -> bool {
        self.damaged.load(Ordering::Relaxed)
    }
    /// Checks a password typed by someone at `ip`, under the per-IP lockout:
    /// refused unchecked while `ip` is locked out, a wrong password counts
    /// towards a lockout, a right one clears the record. Every place that
    /// takes the password (login, password change, opening the terminal)
    /// goes through here, so guesses add up across all of them. Checks run
    /// one at a time: otherwise a burst of parallel guesses would all pass
    /// the lockout test before the first of them was counted. Blocking (the
    /// hash takes a noticeable moment on a Pi), so call it off the async
    /// runtime.
    pub fn check(&self, ip: IpAddr, password: &str) -> Check {
        let _one_at_a_time = self
            .checking
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(remaining) = self.locked_out(ip) {
            return Check::LockedOut(remaining);
        }
        // Held up while we are holding `checking`, so parallel guesses queue
        // behind the delay instead of skipping it.
        std::thread::sleep(global_delay(self.recent_global_failures()));
        if self.verify(password) {
            self.record_success(ip);
            Check::Right
        } else {
            self.record_failure(ip);
            Check::Wrong
        }
    }
    /// Wrong passwords from any address within `GLOBAL_WINDOW`.
    fn recent_global_failures(&self) -> u32 {
        let mut recent = self
            .recent_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while recent.front().is_some_and(|t| t.elapsed() > GLOBAL_WINDOW) {
            recent.pop_front();
        }
        u32::try_from(recent.len()).unwrap_or(u32::MAX)
    }
    /// `Some(remaining)` while `ip` is locked out from too many recent wrong
    /// passwords; clears the lockout itself once it has expired.
    pub fn locked_out(&self, ip: IpAddr) -> Option<Duration> {
        let mut attempts = self.attempts.lock().unwrap();
        let until = attempts.get(&ip)?.locked_until?;
        let now = Instant::now();
        if until > now {
            Some(until - now)
        } else {
            attempts.remove(&ip);
            None
        }
    }
    /// Counts one wrong password from `ip`, starting a lockout once it
    /// reaches `MAX_ATTEMPTS`.
    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        self.recent_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(now);
        let mut attempts = self.attempts.lock().unwrap();
        let a = attempts.entry(ip).or_default();
        // A typo weeks ago must not count towards today's lockout.
        if a.last_failure
            .is_some_and(|t| now.duration_since(t) > LOCKOUT)
        {
            a.failures = 0;
        }
        a.last_failure = Some(now);
        a.failures += 1;
        if a.failures >= MAX_ATTEMPTS {
            a.locked_until = Some(now + LOCKOUT);
        }
    }
    /// A right password clears that IP's record — a fat-fingered attempt or
    /// two shouldn't linger against someone who then logs in correctly.
    pub fn record_success(&self, ip: IpAddr) {
        self.attempts.lock().unwrap().remove(&ip);
    }
    /// A password is set, or the file that should hold it is damaged (then
    /// the login form is the right thing to show, never the first-run setup).
    pub fn configured(&self) -> bool {
        self.damaged() || self.creds.lock().unwrap().is_some()
    }
    fn new_credentials(password: &str) -> Result<Credentials> {
        let salt = random_bytes(SALT_LEN)?;
        Ok(Credentials {
            salt: to_hex(&salt),
            hash: to_hex(&hash(password, &salt)),
            system_synced: false,
        })
    }
    /// Writes `creds` (atomically, readable by root only) and makes them the
    /// current ones. Callers hold the `creds` lock.
    fn store(&self, slot: &mut Option<Credentials>, creds: Credentials) -> Result<()> {
        crate::settings::atomic_write_mode(
            &self.path,
            serde_json::to_string_pretty(&creds)?.as_bytes(),
            0o600,
        )
        .with_context(|| format!("write {:?}", self.path))?;
        *slot = Some(creds);
        self.damaged.store(false, Ordering::Relaxed);
        Ok(())
    }
    /// Sets the password — a change (the first-time setup goes through
    /// `set_password_if_unset`) — and signs every existing session out,
    /// including the caller's own (the API layer logs the caller back in
    /// right after).
    pub fn set_password(&self, password: &str) -> Result<()> {
        let creds = Self::new_credentials(password)?;
        self.store(&mut self.creds.lock().unwrap(), creds)?;
        self.sessions.lock().unwrap().clear();
        Ok(())
    }
    /// First-time setup: stores `password` only if nothing is configured yet,
    /// as one step, so two first visits at the same moment cannot both win.
    /// `Ok(false)` means someone else got there first.
    pub fn set_password_if_unset(&self, password: &str) -> Result<bool> {
        let creds = Self::new_credentials(password)?;
        let mut slot = self.creds.lock().unwrap();
        if slot.is_some() || self.damaged() {
            return Ok(false);
        }
        self.store(&mut slot, creds)?;
        self.sessions.lock().unwrap().clear();
        Ok(true)
    }
    /// The Pi's root/SSH password does not yet match the control password.
    pub fn needs_system_sync(&self) -> bool {
        self.creds
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|c| !c.system_synced)
    }
    /// Records that the Pi's root/SSH password now matches the control
    /// password. Does nothing when the password was changed meanwhile.
    pub fn mark_system_synced(&self, password: &str) -> Result<()> {
        let mut slot = self.creds.lock().unwrap();
        let Some(mut creds) = slot.clone() else {
            return Ok(());
        };
        let (Some(salt), Some(want)) = (from_hex(&creds.salt), from_hex(&creds.hash)) else {
            return Ok(());
        };
        if !ct_eq(&hash(password, &salt), &want) {
            return Ok(());
        }
        creds.system_synced = true;
        self.store(&mut slot, creds)
    }
    pub fn verify(&self, password: &str) -> bool {
        let Some(creds) = self.creds.lock().unwrap().clone() else {
            return false;
        };
        let (Some(salt), Some(want)) = (from_hex(&creds.salt), from_hex(&creds.hash)) else {
            return false;
        };
        ct_eq(&hash(password, &salt), &want)
    }
    /// Issues a fresh session token; the caller sets it as the session cookie.
    pub fn login(&self) -> Result<String> {
        let token = random_token()?;
        self.sessions.lock().unwrap().insert(token.clone());
        Ok(token)
    }
    pub fn logout(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }
    pub fn authenticated(&self, token: Option<&str>) -> bool {
        token.is_some_and(|t| self.sessions.lock().unwrap().contains(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ungoggled-auth-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn store(label: &str) -> Store {
        Store::open(&scratch(label)).unwrap()
    }

    #[test]
    fn unconfigured_until_a_password_is_set() {
        let store = store("unconfigured");
        assert!(!store.configured());
        assert!(!store.verify("anything"));
        store.set_password("hunter2").unwrap();
        assert!(store.configured());
    }

    #[test]
    fn verify_only_accepts_the_right_password() {
        let store = store("verify");
        store.set_password("correct horse").unwrap();
        assert!(store.verify("correct horse"));
        assert!(!store.verify("wrong"));
        assert!(!store.verify(""));
    }

    #[test]
    fn login_token_is_authenticated_until_logout() {
        let store = store("login");
        store.set_password("x").unwrap();
        let token = store.login().unwrap();
        assert!(store.authenticated(Some(&token)));
        assert!(!store.authenticated(Some("not-a-real-token")));
        assert!(!store.authenticated(None));
        store.logout(&token);
        assert!(!store.authenticated(Some(&token)));
    }

    #[test]
    fn changing_the_password_signs_out_every_session() {
        let store = store("change");
        store.set_password("first").unwrap();
        let token = store.login().unwrap();
        assert!(store.authenticated(Some(&token)));
        store.set_password("second").unwrap();
        assert!(!store.authenticated(Some(&token)));
        assert!(!store.verify("first"));
        assert!(store.verify("second"));
    }

    #[test]
    fn locks_out_after_max_attempts() {
        let store = store("lockout");
        store.set_password("correct").unwrap();
        let ip: IpAddr = "203.0.113.1".parse().unwrap();
        assert!(store.locked_out(ip).is_none());
        for _ in 0..MAX_ATTEMPTS - 1 {
            store.record_failure(ip);
            assert!(store.locked_out(ip).is_none());
        }
        store.record_failure(ip);
        let remaining = store.locked_out(ip).expect("should be locked out now");
        assert!(remaining <= LOCKOUT && remaining > Duration::ZERO);
    }

    #[test]
    fn a_right_password_clears_the_lockout_record() {
        let store = store("lockout-clears");
        store.set_password("correct").unwrap();
        let ip: IpAddr = "203.0.113.2".parse().unwrap();
        for _ in 0..MAX_ATTEMPTS - 1 {
            store.record_failure(ip);
        }
        store.record_success(ip);
        store.record_failure(ip);
        assert!(
            store.locked_out(ip).is_none(),
            "one more failure after a reset should not re-trigger the lockout"
        );
    }

    #[test]
    fn lockouts_are_tracked_separately_per_ip() {
        let store = store("lockout-per-ip");
        store.set_password("correct").unwrap();
        let attacker: IpAddr = "203.0.113.3".parse().unwrap();
        let owner: IpAddr = "203.0.113.4".parse().unwrap();
        for _ in 0..MAX_ATTEMPTS {
            store.record_failure(attacker);
        }
        assert!(store.locked_out(attacker).is_some());
        assert!(store.locked_out(owner).is_none());
    }

    #[test]
    fn check_counts_wrong_passwords_and_refuses_while_locked_out() {
        let store = store("check");
        store.set_password("correct").unwrap();
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        for _ in 0..MAX_ATTEMPTS - 1 {
            assert_eq!(store.check(ip, "wrong"), Check::Wrong);
        }
        assert_eq!(store.check(ip, "correct"), Check::Right);
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(store.check(ip, "wrong"), Check::Wrong);
        }
        // Locked out: even the right password isn't looked at now.
        assert!(matches!(
            store.check(ip, "correct"),
            Check::LockedOut(d) if d > Duration::ZERO
        ));
    }

    #[test]
    fn parallel_guesses_cannot_outrun_the_lockout() {
        let store = store("check-parallel");
        store.set_password("correct").unwrap();
        let ip: IpAddr = "203.0.113.6".parse().unwrap();
        let outcomes: Vec<Check> = std::thread::scope(|s| {
            let guesses: Vec<_> = (0..4 * MAX_ATTEMPTS)
                .map(|i| {
                    let store = &store;
                    s.spawn(move || store.check(ip, &format!("guess-{i}")))
                })
                .collect();
            guesses.into_iter().map(|g| g.join().unwrap()).collect()
        });
        let tried = outcomes.iter().filter(|o| **o == Check::Wrong).count();
        assert_eq!(tried, MAX_ATTEMPTS as usize, "{outcomes:?}");
    }

    #[test]
    fn wrong_passwords_stop_counting_after_the_lockout_time() {
        let store = store("decay");
        store.set_password("correct").unwrap();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        for _ in 0..MAX_ATTEMPTS - 1 {
            store.record_failure(ip);
        }
        // Weeks later: the old failures are forgotten, one typo is just one.
        store
            .attempts
            .lock()
            .unwrap()
            .get_mut(&ip)
            .unwrap()
            .last_failure = Some(Instant::now() - LOCKOUT - Duration::from_secs(1));
        store.record_failure(ip);
        assert!(store.locked_out(ip).is_none());
    }

    #[test]
    fn global_delay_starts_late_doubles_and_is_capped() {
        assert_eq!(global_delay(0), Duration::ZERO);
        assert_eq!(global_delay(GLOBAL_FREE), Duration::ZERO);
        assert_eq!(global_delay(GLOBAL_FREE + 1), Duration::from_secs(2));
        assert_eq!(global_delay(GLOBAL_FREE + 2), Duration::from_secs(4));
        assert_eq!(global_delay(GLOBAL_FREE + 40), GLOBAL_MAX_DELAY);
    }

    #[test]
    fn failures_from_every_address_add_up_and_old_ones_drop_out() {
        let store = store("global");
        store.set_password("correct").unwrap();
        for i in 0..30u8 {
            store.record_failure(IpAddr::from([203, 0, 113, i]));
        }
        assert_eq!(store.recent_global_failures(), 30);
        {
            let mut recent = store.recent_failures.lock().unwrap();
            for t in recent.iter_mut().take(25) {
                *t = Instant::now() - GLOBAL_WINDOW - Duration::from_secs(1);
            }
        }
        assert_eq!(store.recent_global_failures(), 5);
    }

    #[test]
    fn the_server_enforces_the_password_rules() {
        assert!(check_new_password("12345678").is_ok());
        assert!(check_new_password("correct horse battery").is_ok());
        assert!(check_new_password("short").is_err());
        assert!(
            check_new_password("        ").is_ok(),
            "spaces are characters"
        );
        assert!(check_new_password("line\nbreak-here").is_err());
        assert!(check_new_password("tab\tinside-it").is_err());
        // Counted in characters, not bytes.
        assert!(check_new_password("äöüäöüäö").is_ok());
        assert!(check_new_password("äöüäöüä").is_err());
    }

    #[test]
    fn first_setup_only_wins_once() {
        let store = store("initial");
        assert!(store.set_password_if_unset("first-password").unwrap());
        assert!(!store.set_password_if_unset("second-password").unwrap());
        assert!(store.verify("first-password"));
        assert!(!store.verify("second-password"));
    }

    #[test]
    fn two_first_visits_at_once_leave_exactly_one_password() {
        let store = store("initial-race");
        let wins: usize = std::thread::scope(|s| {
            let runs: Vec<_> = (0..4)
                .map(|i| {
                    let store = &store;
                    s.spawn(move || {
                        store
                            .set_password_if_unset(&format!("password-{i}"))
                            .unwrap()
                    })
                })
                .collect();
            runs.into_iter()
                .map(|r| usize::from(r.join().unwrap()))
                .sum()
        });
        assert_eq!(wins, 1);
    }

    #[test]
    fn the_system_password_follows_until_marked_done() {
        let dir = scratch("sync");
        let store = Store::open(&dir).unwrap();
        assert!(
            !store.needs_system_sync(),
            "nothing configured, nothing to do"
        );
        store.set_password("sync-me-please").unwrap();
        assert!(store.needs_system_sync());
        // A stale mark (the password was changed meanwhile) must not stick.
        store.mark_system_synced("some-older-password").unwrap();
        assert!(store.needs_system_sync());
        store.mark_system_synced("sync-me-please").unwrap();
        assert!(!store.needs_system_sync());
        assert!(!Store::open(&dir).unwrap().needs_system_sync(), "persisted");
        store.set_password("changed-again-now").unwrap();
        assert!(
            store.needs_system_sync(),
            "a new password is not synced yet"
        );
    }

    #[test]
    fn a_file_from_before_the_sync_existed_still_works_and_is_synced_later() {
        let dir = scratch("legacy");
        let first = Store::open(&dir).unwrap();
        first.set_password("legacy-password").unwrap();
        let text = fs::read_to_string(dir.join("auth.json")).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("system_synced");
        fs::write(dir.join("auth.json"), value.to_string()).unwrap();
        let store = Store::open(&dir).unwrap();
        assert!(store.verify("legacy-password"));
        assert!(store.needs_system_sync());
    }

    #[test]
    fn a_damaged_file_keeps_the_service_up_and_refuses_logins() {
        let dir = scratch("damaged");
        fs::write(dir.join("auth.json"), "{\"salt\": \"ab").unwrap();
        let store = Store::open(&dir).unwrap();
        assert!(store.damaged());
        assert!(
            store.configured(),
            "the login form, never the first-run setup"
        );
        assert!(!store.verify(""));
        assert!(!store.verify("anything"));
        assert!(
            !store.set_password_if_unset("take-over-attempt").unwrap(),
            "a damaged file must not hand the console to whoever asks first"
        );
        // The owner can still repair it deliberately.
        store.set_password("repaired-password").unwrap();
        assert!(!store.damaged());
        assert!(store.verify("repaired-password"));
    }

    #[test]
    fn an_old_world_readable_password_file_is_tightened_when_opened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("tighten");
        let first = Store::open(&dir).unwrap();
        first.set_password("tighten-me-please").unwrap();
        fs::set_permissions(dir.join("auth.json"), fs::Permissions::from_mode(0o644)).unwrap();
        let reopened = Store::open(&dir).unwrap();
        let mode = fs::metadata(dir.join("auth.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(reopened.verify("tighten-me-please"));
    }

    #[test]
    fn the_password_file_is_private_and_written_in_one_piece() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        let store = Store::open(&dir).unwrap();
        store.set_password("private-password").unwrap();
        let mode = fs::metadata(dir.join("auth.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn persists_across_reopening_the_same_directory() {
        let dir = scratch("persist");
        let store = Store::open(&dir).unwrap();
        store.set_password("durable").unwrap();
        let reopened = Store::open(&dir).unwrap();
        assert!(reopened.configured());
        assert!(reopened.verify("durable"));
    }
}
