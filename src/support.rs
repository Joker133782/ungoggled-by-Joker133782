//! A downloadable diagnostics bundle: recent service logs, the update
//! history, hardware diagnostics and the current settings/status, all in one
//! `.tar.gz` — so a problem can be reported, even from a phone, without SSH
//! access to the Pi.
use anyhow::Result;
use flate2::{Compression, write::GzEncoder};
use serde_json::Value;
use std::{process::Command, time::SystemTime};

/// Best-effort: a command this Pi doesn't have (e.g. `vcgencmd` without the
/// firmware tools) just gets recorded as missing, never fails the bundle.
fn run(cmd: &str, args: &[&str]) -> String {
    Command::new("timeout")
        .arg("10")
        .arg(cmd)
        .args(args)
        .output()
        .map(|o| {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            if !o.stderr.is_empty() {
                s.push_str("\n--- stderr ---\n");
                s.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            s
        })
        .unwrap_or_else(|e| format!("Could not run {cmd}: {e:#}"))
}
fn add(archive: &mut tar::Builder<GzEncoder<Vec<u8>>>, name: &str, data: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );
    header.set_cksum();
    archive.append_data(&mut header, name, data)?;
    Ok(())
}
/// Synchronous and a few seconds of subprocess calls (journalctl over
/// thousands of lines, mainly); run this off the async runtime.
pub fn build(settings: &Value, status: &Value) -> Result<Vec<u8>> {
    let mut archive = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    add(
        &mut archive,
        "status.json",
        serde_json::to_string_pretty(status)?.as_bytes(),
    )?;
    add(
        &mut archive,
        "settings.json",
        serde_json::to_string_pretty(settings)?.as_bytes(),
    )?;
    add(
        &mut archive,
        "diagnostics.json",
        serde_json::to_string_pretty(&crate::doctor())?.as_bytes(),
    )?;
    let releases = crate::update::releases_information(&crate::update::Paths::default());
    add(
        &mut archive,
        "releases.json",
        serde_json::to_string_pretty(&releases)?.as_bytes(),
    )?;
    add(
        &mut archive,
        "journal-dji-hdmi.log",
        run(
            "journalctl",
            &["-u", "dji-hdmi", "-n", "2000", "--no-pager"],
        )
        .as_bytes(),
    )?;
    add(
        &mut archive,
        "journal-update-recovery.log",
        run(
            "journalctl",
            &["-u", "ungoggled-update-recovery", "-n", "500", "--no-pager"],
        )
        .as_bytes(),
    )?;
    let system_info = format!(
        "uname -a:\n{}\n\ndf -h:\n{}\n\nfree -h:\n{}\n\nvcgencmd measure_temp:\n{}\n",
        run("uname", &["-a"]),
        run("df", &["-h"]),
        run("free", &["-h"]),
        run("vcgencmd", &["measure_temp"]),
    );
    add(&mut archive, "system-info.txt", system_info.as_bytes())?;
    Ok(archive.into_inner()?.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_valid_gzip_tar_with_the_expected_entries() {
        let bytes = build(&serde_json::json!({"a":1}), &serde_json::json!({"b":2})).unwrap();
        assert!(bytes.starts_with(&[0x1f, 0x8b]), "not gzip-magic-prefixed");
        let decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut archive = tar::Archive::new(decoder);
        let names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        for expected in [
            "status.json",
            "settings.json",
            "diagnostics.json",
            "releases.json",
            "journal-dji-hdmi.log",
            "journal-update-recovery.log",
            "system-info.txt",
        ] {
            assert!(names.contains(&expected.to_owned()), "missing {expected}");
        }
    }
}
