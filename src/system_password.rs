//! Keeps the Pi's own `root` login (SSH and the console terminal) on the same
//! password as the control interface: one password to remember, and no
//! well-known default left behind once the owner has chosen their own. Until
//! then both are `ungoggled`, as set by the flash image.
//!
//! The password goes to `chpasswd` on its standard input (never on a command
//! line, where every process could read it). The service runs as root; on a
//! machine where it does not (a development run) this simply fails and the
//! control password still works.
use anyhow::{Context, Result, bail};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

/// Set (to anything) when this server is only a throwaway instance inside a
/// system whose passwords must stay as they are: the image build's smoke
/// test (`scripts/check-image.sh`) runs the real server in the image being
/// built and logs in with a throwaway password, which must not become the
/// image's root password.
pub const KEEP_ENV: &str = "UNGOGGLED_KEEP_SYSTEM_PASSWORD";

/// Sets the password of `root` to `password`.
pub fn set_root(password: &str) -> Result<()> {
    if std::env::var_os(KEEP_ENV).is_some() {
        return Ok(());
    }
    set_root_with(Path::new("chpasswd"), password)
}

/// `set_root` with the `chpasswd` program to run (replaced in tests).
fn set_root_with(chpasswd: &Path, password: &str) -> Result<()> {
    // `chpasswd` reads `user:password` lines: a line break would start a
    // second line, i.e. let a password set another account's password.
    if password.is_empty() || password.chars().any(char::is_control) {
        bail!("password cannot be used as a system password");
    }
    let mut child = Command::new("timeout")
        .arg("30")
        .arg(chpasswd)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("start chpasswd")?;
    let mut stdin = child.stdin.take().context("chpasswd stdin")?;
    let written = stdin.write_all(format!("root:{password}\n").as_bytes());
    drop(stdin);
    let output = child.wait_with_output().context("wait for chpasswd")?;
    written.context("write the password to chpasswd")?;
    if !output.status.success() {
        // chpasswd's own message names the problem and never contains the
        // password, so it is safe to pass on.
        bail!(
            "chpasswd failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    /// A stand-in `chpasswd` that records its standard input and arguments.
    fn fake(label: &str, exit: i32) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "ungoggled-syspw-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let program = dir.join("chpasswd");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\ncat > \"$(dirname \"$0\")/stdin\"\necho \"$#\" > \"$(dirname \"$0\")/argc\"\nexit {exit}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        (program, dir)
    }

    #[test]
    fn the_password_reaches_chpasswd_on_stdin_for_root_only() {
        let (program, dir) = fake("ok", 0);
        set_root_with(&program, "p:a ss'w\"ord$1").unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("stdin")).unwrap(),
            "root:p:a ss'w\"ord$1\n",
            "colons and quotes are part of the password"
        );
        assert_eq!(
            fs::read_to_string(dir.join("argc")).unwrap().trim(),
            "0",
            "nothing secret on a command line"
        );
    }

    #[test]
    fn a_password_that_could_start_a_second_line_is_refused_untouched() {
        let (program, dir) = fake("lines", 0);
        assert!(set_root_with(&program, "first\nbin:second").is_err());
        assert!(set_root_with(&program, "carriage\rreturn").is_err());
        assert!(set_root_with(&program, "").is_err());
        assert!(!dir.join("stdin").exists(), "chpasswd was never started");
    }

    #[test]
    fn a_failing_chpasswd_is_an_error() {
        let (program, _dir) = fake("fail", 1);
        assert!(set_root_with(&program, "valid-password").is_err());
    }

    #[test]
    fn a_missing_chpasswd_is_an_error_not_a_panic() {
        assert!(set_root_with(Path::new("/nonexistent/chpasswd"), "valid-password").is_err());
    }
}
