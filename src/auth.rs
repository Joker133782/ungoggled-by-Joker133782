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
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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

#[derive(Default)]
struct Attempts {
    failures: u32,
    locked_until: Option<Instant>,
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
    /// Held for the whole of one `check`; see there.
    checking: Arc<Mutex<()>>,
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("auth.json");
        let creds = if path.exists() {
            Some(
                serde_json::from_str(
                    &fs::read_to_string(&path).with_context(|| format!("read {path:?}"))?,
                )
                .with_context(|| format!("parse {path:?}"))?,
            )
        } else {
            None
        };
        Ok(Self {
            path,
            creds: Arc::new(Mutex::new(creds)),
            sessions: Arc::new(Mutex::new(HashSet::new())),
            attempts: Arc::new(Mutex::new(HashMap::new())),
            checking: Arc::new(Mutex::new(())),
        })
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
        if self.verify(password) {
            self.record_success(ip);
            Check::Right
        } else {
            self.record_failure(ip);
            Check::Wrong
        }
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
        let mut attempts = self.attempts.lock().unwrap();
        let a = attempts.entry(ip).or_default();
        a.failures += 1;
        if a.failures >= MAX_ATTEMPTS {
            a.locked_until = Some(Instant::now() + LOCKOUT);
        }
    }
    /// A right password clears that IP's record — a fat-fingered attempt or
    /// two shouldn't linger against someone who then logs in correctly.
    pub fn record_success(&self, ip: IpAddr) {
        self.attempts.lock().unwrap().remove(&ip);
    }
    pub fn configured(&self) -> bool {
        self.creds.lock().unwrap().is_some()
    }
    /// Sets the password — first-time setup or a change, no distinction —
    /// and signs every existing session out, including the caller's own
    /// (the API layer logs the caller back in right after, for a change).
    pub fn set_password(&self, password: &str) -> Result<()> {
        let salt = random_bytes(SALT_LEN)?;
        let creds = Credentials {
            salt: to_hex(&salt),
            hash: to_hex(&hash(password, &salt)),
        };
        fs::write(&self.path, serde_json::to_string_pretty(&creds)?)
            .with_context(|| format!("write {:?}", self.path))?;
        *self.creds.lock().unwrap() = Some(creds);
        self.sessions.lock().unwrap().clear();
        Ok(())
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
    fn persists_across_reopening_the_same_directory() {
        let dir = scratch("persist");
        let store = Store::open(&dir).unwrap();
        store.set_password("durable").unwrap();
        let reopened = Store::open(&dir).unwrap();
        assert!(reopened.configured());
        assert!(reopened.verify("durable"));
    }
}
