use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub ssid: String,
    pub password: String,
}
pub fn validate(c: &Change) -> Result<()> {
    if c.ssid.is_empty() || c.ssid.len() > 32 || c.ssid.chars().any(char::is_control) {
        bail!("SSID must contain 1–32 UTF-8 bytes, without control characters");
    }
    if !(8..=63).contains(&c.password.len()) || !c.password.bytes().all(|b| (32..=126).contains(&b))
    {
        bail!("Password must contain 8–63 printable ASCII characters");
    }
    Ok(())
}
fn path() -> Option<(PathBuf, bool)> {
    let nm = PathBuf::from("/etc/NetworkManager/system-connections/dji-hdmi.nmconnection");
    if nm.exists() {
        return Some((nm, true));
    }
    let ap = PathBuf::from("/etc/dji-hdmi/hostapd.conf");
    ap.exists().then_some((ap, false))
}
fn field<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    s.lines().find_map(|l| l.strip_prefix(key))
}
fn decode_hex(s: &str) -> Option<String> {
    let b: Option<Vec<_>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(b?).ok()
}
/// The access point's SSID from its config file's text (`nm`: a
/// NetworkManager keyfile, else a hostapd.conf).
fn parse_ssid(s: &str, nm: bool) -> Option<String> {
    if nm {
        field(s, "ssid=").and_then(|s| {
            if s.ends_with(';') {
                let b: Option<Vec<u8>> = s
                    .trim_end_matches(';')
                    .split(';')
                    .map(|n| n.parse().ok())
                    .collect();
                String::from_utf8(b?).ok()
            } else {
                Some(s.to_owned())
            }
        })
    } else {
        field(s, "ssid2=")
            .and_then(decode_hex)
            .or_else(|| field(s, "ssid=").map(str::to_owned))
    }
}
/// The access point's password from the same text; `None` for an open one.
/// Undoes the keyfile string escaping `apply` (and NetworkManager itself)
/// writes: `\s` for a space, `\\` for a backslash, and so on.
fn parse_password(s: &str, nm: bool) -> Option<String> {
    if !nm {
        return field(s, "wpa_passphrase=").map(str::to_owned);
    }
    let raw = field(s, "psk=")?;
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    Some(out)
}
pub fn status() -> Value {
    let Some((p, nm)) = path() else {
        return json!({"available":false});
    };
    let s = fs::read_to_string(p).unwrap_or_default();
    let ssid = parse_ssid(&s, nm);
    json!({"available":true,"ssid":ssid,"backend":if nm {"NetworkManager"}else{"hostapd"}})
}
/// What a spectator's phone needs to join the Pi's own Wi-Fi, for the share
/// sheet's Wi-Fi QR code — the access point's current SSID and password,
/// read from the same config `status` reads. Only ever served to a logged-in
/// owner (see `api.rs`), and only fetched when the share sheet opens rather
/// than with the Wi-Fi status every page polls. While the Pi has joined
/// another network as a client the access point is down: then only that
/// network's name is returned, never its password.
pub fn share() -> Value {
    if wlan0_connection().as_deref() == Some(CLIENT_CONNECTION) {
        return json!({"mode":"client","ssid":client_ssid()});
    }
    let Some((p, nm)) = path() else {
        return json!({"mode":"none"});
    };
    let s = fs::read_to_string(p).unwrap_or_default();
    share_from(&s, nm)
}
fn share_from(s: &str, nm: bool) -> Value {
    let password = parse_password(s, nm).filter(|p| !p.is_empty());
    json!({
        "mode": "ap",
        "ssid": parse_ssid(s, nm),
        "security": if password.is_some() { "WPA" } else { "nopass" },
        "password": password,
    })
}
pub fn apply(c: Change) -> Result<()> {
    validate(&c)?;
    let (path, nm) = path().context("No managed access point configured")?;
    let original = fs::read_to_string(&path)?;
    let next = if nm {
        let ssid = c.ssid.bytes().map(|b| format!("{b};")).collect::<String>();
        // Keyfile strings escape backslashes and leading/trailing spaces.
        let password = c.password.replace('\\', "\\\\").replace(' ', "\\s");
        replace_fields(&original, &[("ssid=", ssid), ("psk=", password)])
    } else {
        let ssid = c
            .ssid
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let s = original
            .lines()
            .filter(|l| {
                !l.starts_with("ssid=")
                    && !l.starts_with("ssid2=")
                    && !l.starts_with("wpa_passphrase=")
                    && !l.starts_with("wpa_psk=")
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("{s}\nssid2={ssid}\nwpa_passphrase={}\n", c.password)
    };
    private_write(&path, next.as_bytes())?;
    let result = restart(nm);
    if result.is_err() {
        private_write(&path, original.as_bytes())?;
        let _ = restart(nm);
    }
    result
}
fn replace_fields(s: &str, fields: &[(&str, String)]) -> String {
    s.lines()
        .map(|l| {
            fields
                .iter()
                .find(|(key, _)| l.starts_with(key))
                .map_or_else(|| l.to_owned(), |(key, v)| format!("{key}{v}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temp = path.with_extension("new");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)?;
    f.set_permissions(fs::Permissions::from_mode(0o600))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(temp, path)?;
    Ok(())
}
fn restart(nm: bool) -> Result<()> {
    if nm {
        run("nmcli", &["connection", "reload"])?;
        run("nmcli", &["--wait", "20", "connection", "up", "dji-hdmi"])
    } else {
        run("systemctl", &["restart", "dji-hdmi-ap.service"])
    }
}
fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new("timeout")
        .arg("30")
        .arg(program)
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!("Wi-Fi could not be applied; previous configuration restored");
    }
    Ok(())
}
/// The access point profile every image ships with (see `path` above); the
/// one name constant that already exists is intentionally not duplicated
/// here.
const AP_CONNECTION: &str = "dji-hdmi";
/// Created on demand by `join`; never shipped with the image.
const CLIENT_CONNECTION: &str = "dji-hdmi-client";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    pub ssid: String,
    pub password: String,
}
pub fn validate_join(c: &JoinRequest) -> Result<()> {
    if c.ssid.is_empty() || c.ssid.len() > 32 || c.ssid.chars().any(char::is_control) {
        bail!("SSID must contain 1–32 UTF-8 bytes, without control characters");
    }
    if !c.password.is_empty()
        && (!(8..=63).contains(&c.password.len())
            || !c.password.bytes().all(|b| (32..=126).contains(&b)))
    {
        bail!("Password must be empty (open network) or 8–63 printable ASCII characters");
    }
    Ok(())
}
/// Unlike `run`, never echoes `args` in the error: `join` passes the Wi-Fi
/// password as one, and nmcli's own stderr doesn't echo it back either.
fn exec(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new("timeout")
        .arg("30")
        .arg(program)
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!("{program} command failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
/// Join an existing Wi-Fi network as a client, replacing the access point on
/// the Pi's one radio until `leave_client` runs or `watch` falls back on a
/// connection that stays unhealthy. `always_ap` mirrors the setting of the
/// same name: when true, this profile is never allowed to autoconnect on a
/// future boot, so a Pi taken flying can't get stuck hunting for a home
/// network that's out of range — only an explicit join brings it up again.
pub fn join(c: &JoinRequest, always_ap: bool) -> Result<()> {
    validate_join(c)?;
    if !PathBuf::from("/etc/NetworkManager/system-connections/dji-hdmi.nmconnection").exists() {
        bail!("Client mode needs the NetworkManager-based access point backend");
    }
    let _ = exec("nmcli", &["connection", "delete", CLIENT_CONNECTION]);
    let mut args = vec![
        "connection",
        "add",
        "type",
        "wifi",
        "con-name",
        CLIENT_CONNECTION,
        "ifname",
        "wlan0",
        "ssid",
        c.ssid.as_str(),
        "connection.autoconnect",
        if always_ap { "false" } else { "true" },
        "connection.autoconnect-priority",
        "200",
        "ipv4.method",
        "auto",
        "ipv6.method",
        "disabled",
    ];
    if !c.password.is_empty() {
        args.extend([
            "wifi-sec.key-mgmt",
            "wpa-psk",
            "wifi-sec.psk",
            c.password.as_str(),
        ]);
    }
    exec("nmcli", &args)?;
    // --wait bounds nmcli's own wait so a phantom SSID reports failure
    // itself within a predictable time, rather than relying on the external
    // `timeout` wrapper to kill it — a killed nmcli client doesn't
    // necessarily tell NetworkManager to abandon the activation it already
    // asked for, which left the radio stuck "connecting" indefinitely
    // (found 2026-10-31: a scan hung and the status never left "connecting"
    // after a deliberately-bad test join). The explicit `down` below is the
    // actual fix for that: it cancels the activation regardless of whether
    // nmcli itself reported success, failure, or just timed out.
    let result = exec(
        "nmcli",
        &["--wait", "25", "connection", "up", CLIENT_CONNECTION],
    );
    if result.is_err() {
        let _ = exec("nmcli", &["connection", "down", CLIENT_CONNECTION]);
        let _ = exec("nmcli", &["connection", "delete", CLIENT_CONNECTION]);
        let _ = exec("nmcli", &["connection", "up", AP_CONNECTION]);
        // Specific and reassuring, not the generic `exec` message: this
        // command's own args never include the password, so echoing detail
        // here (unlike the `connection add` step above) can't leak it, and
        // the one thing this error needs to say is that recovery already
        // happened — the error otherwise reads like an ongoing problem.
        bail!(
            "Could not join \"{}\" — wrong password, out of range, or the \
             network doesn't exist. Already back on the access point.",
            c.ssid
        );
    }
    Ok(())
}
/// Keeps an already-saved client profile's autoconnect flag in sync with the
/// `wifi_always_ap` setting, for when it's toggled after a join rather than
/// before one. A no-op if no client profile has been created yet.
pub fn set_client_autoconnect(always_ap: bool) {
    let _ = exec(
        "nmcli",
        &[
            "connection",
            "modify",
            CLIENT_CONNECTION,
            "connection.autoconnect",
            if always_ap { "false" } else { "true" },
        ],
    );
}
/// Deactivates the client profile, if any, and brings the access point back.
/// The explicit `down` first matters even when the client profile looks
/// inactive already (see the comment in `join`): it's what actually tells
/// NetworkManager to give up on a stuck activation, not just requesting the
/// access point and hoping that preempts it.
pub fn leave_client() -> Result<()> {
    let _ = exec("nmcli", &["connection", "down", CLIENT_CONNECTION]);
    exec("nmcli", &["connection", "up", AP_CONNECTION]).map(|_| ())
}
/// The connection NetworkManager currently has active on wlan0, by name.
fn wlan0_connection() -> Option<String> {
    let out = exec(
        "nmcli",
        &["-t", "-f", "DEVICE,NAME", "connection", "show", "--active"],
    )
    .ok()?;
    out.lines().find_map(|l| {
        let (device, name) = l.split_once(':')?;
        (device == "wlan0").then(|| name.to_owned())
    })
}
fn client_ip() -> Option<String> {
    exec(
        "nmcli",
        &["-g", "IP4.ADDRESS", "connection", "show", CLIENT_CONNECTION],
    )
    .ok()
    .filter(|s| !s.is_empty())
}
fn client_ssid() -> Option<String> {
    exec(
        "nmcli",
        &[
            "-g",
            "802-11-wireless.ssid",
            "connection",
            "show",
            CLIENT_CONNECTION,
        ],
    )
    .ok()
    .filter(|s| !s.is_empty())
}
/// For the web UI and the on-screen status overlay: whether the client
/// profile is the one active on wlan0 right now, which network it's for,
/// and its IP address once DHCP has handed one out.
pub fn client_status() -> Value {
    let active = wlan0_connection().as_deref() == Some(CLIENT_CONNECTION);
    json!({
        "active":active,
        "ssid":client_ssid(),
        "ip":if active { client_ip() } else { None },
    })
}
/// Splits one line of `nmcli -t` output on unescaped `:` and undoes its
/// backslash escaping (`\:` and `\\`) — needed because an SSID or password
/// can itself contain a colon, which terse mode otherwise uses as the field
/// separator.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ':' => fields.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    fields.push(current);
    fields
}
/// Visible networks right now, strongest signal first, for the web UI's
/// "pick a network" list — never the Pi's own access point. A dual-band
/// router advertising the same SSID on both bands is merged into one entry
/// (its stronger band's signal); joining by SSID lets NetworkManager itself
/// pick the band to use.
///
/// Deliberately doesn't force `--rescan yes`: a full active rescan has to
/// visit every supported channel, including 5GHz DFS ones that each need a
/// multi-second passive radar check before they can even be probed — slow
/// enough to make this hang well past a reasonable request timeout once the
/// access point itself also moved to 5GHz. NetworkManager already rescans
/// periodically on its own; this just reads that already-known list, which
/// is near-instant and good enough for picking a network to join.
pub fn scan() -> Result<Value> {
    let own_ssid = status()["ssid"].as_str().map(str::to_owned);
    let out = exec(
        "nmcli",
        &["-t", "-f", "SSID,SIGNAL,SECURITY", "device", "wifi", "list"],
    )?;
    let mut best: std::collections::HashMap<String, (u8, bool)> = std::collections::HashMap::new();
    for line in out.lines() {
        let f = split_terse(line);
        let (Some(ssid), Some(signal), Some(security)) = (f.first(), f.get(1), f.get(2)) else {
            continue;
        };
        if ssid.is_empty() || Some(ssid.as_str()) == own_ssid.as_deref() {
            continue;
        }
        let signal: u8 = signal.parse().unwrap_or(0);
        let secured = !security.is_empty();
        best.entry(ssid.clone())
            .and_modify(|e| {
                if signal > e.0 {
                    *e = (signal, secured);
                }
            })
            .or_insert((signal, secured));
    }
    let mut networks: Vec<_> = best.into_iter().collect();
    networks.sort_by_key(|a| std::cmp::Reverse(a.1.0));
    Ok(json!({
        "networks": networks
            .into_iter()
            .map(|(ssid, (signal, secured))| json!({"ssid":ssid,"signal":signal,"secured":secured}))
            .collect::<Vec<_>>()
    }))
}
/// Runs for the lifetime of the service. While the client profile is the one
/// actually active on wlan0, falls back to the access point once it has had
/// no IP address for 30 continuous seconds — e.g. a wrong password, or the
/// Pi leaving range of a home network mid-flight. An intentional
/// `leave_client` call already makes the access point active again, so this
/// naturally stops watching the moment that happens.
///
/// `busy` is the same flag the API handlers use around `join`/`leave_client`
/// (`App::wifi_busy`): skipped while it's held, so this never fires its own
/// `leave_client` in the middle of a user-initiated one and issues two
/// concurrent `nmcli` operations against the same radio.
pub fn watch(shutdown: &AtomicBool, busy: &AtomicBool, error: &Mutex<Option<String>>) {
    let mut unhealthy_since: Option<Instant> = None;
    while !shutdown.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(2));
        if busy.load(Ordering::Relaxed) {
            continue;
        }
        if wlan0_connection().as_deref() != Some(CLIENT_CONNECTION) || client_ip().is_some() {
            unhealthy_since = None;
            continue;
        }
        if unhealthy_since.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(30) {
            if busy.swap(true, Ordering::SeqCst) {
                continue;
            }
            let _ = leave_client();
            busy.store(false, Ordering::Relaxed);
            *error.lock().unwrap() =
                Some("Lost the Wi-Fi connection for 30 seconds — back on the access point.".into());
            unhealthy_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_wifi_without_shell_interpretation() {
        assert!(
            validate(&Change {
                ssid: "My Wi-Fi".into(),
                password: "12345678".into()
            })
            .is_ok()
        );
        assert!(
            validate(&Change {
                ssid: "bad\nssid".into(),
                password: "12345678".into()
            })
            .is_err()
        );
        assert!(
            validate(&Change {
                ssid: "x".into(),
                password: "short".into()
            })
            .is_err()
        );
        assert!(
            validate(&Change {
                ssid: "é".repeat(17),
                password: "12345678".into()
            })
            .is_err()
        );
        assert_eq!(decode_hex("4d792057694669"), Some("My WiFi".into()));
    }
    #[test]
    fn share_reads_the_access_point_credentials_back() {
        // As provisioned (plain SSID) ...
        let image =
            "[wifi]\nmode=ap\nssid=ungoggled\n\n[wifi-security]\nkey-mgmt=wpa-psk\npsk=ungoggled\n";
        let s = share_from(image, true);
        assert_eq!(s["mode"], "ap");
        assert_eq!(s["ssid"], "ungoggled");
        assert_eq!(s["password"], "ungoggled");
        assert_eq!(s["security"], "WPA");
        // ... and as rewritten by `apply`: byte-list SSID, escaped password.
        let applied = replace_fields(
            image,
            &[
                (
                    "ssid=",
                    "My Wi-Fi;".bytes().map(|b| format!("{b};")).collect(),
                ),
                (
                    "psk=",
                    r"a b\c d;e".replace('\\', "\\\\").replace(' ', "\\s"),
                ),
            ],
        );
        let s = share_from(&applied, true);
        assert_eq!(s["ssid"], "My Wi-Fi;");
        assert_eq!(s["password"], r"a b\c d;e");
        // hostapd backend.
        let hostapd = "interface=wlan0\nssid2=4d792057694669\nwpa_passphrase=pa ss;word\n";
        let s = share_from(hostapd, false);
        assert_eq!(s["ssid"], "My WiFi");
        assert_eq!(s["password"], "pa ss;word");
        // An open access point has no password to share.
        let s = share_from("[wifi]\nssid=open\n", true);
        assert_eq!(s["security"], "nopass");
        assert!(s["password"].is_null());
    }
    #[test]
    fn splits_terse_output_and_unescapes_colons() {
        assert_eq!(
            split_terse("My\\:Network:62:WPA2"),
            vec!["My:Network", "62", "WPA2"]
        );
        assert_eq!(split_terse("Open Network::"), vec!["Open Network", "", ""]);
    }
    #[test]
    fn validates_join_request_allows_open_networks() {
        assert!(
            validate_join(&JoinRequest {
                ssid: "Home".into(),
                password: "".into()
            })
            .is_ok()
        );
        assert!(
            validate_join(&JoinRequest {
                ssid: "Home".into(),
                password: "short".into()
            })
            .is_err()
        );
        assert!(
            validate_join(&JoinRequest {
                ssid: "".into(),
                password: "12345678".into()
            })
            .is_err()
        );
    }
}
