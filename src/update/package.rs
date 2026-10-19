use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Component, Path},
};

pub const MAX_UPLOAD: usize = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 256 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileInfo {
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub product: String,
    pub version: String,
    pub minimum_updater: String,
    pub settings_schema: u32,
    pub files: BTreeMap<String, FileInfo>,
}
impl Manifest {
    pub fn release_id(&self) -> Result<String> {
        Ok(format!(
            "{}-{}",
            self.version,
            &hex(&Sha256::digest(serde_json::to_vec(self)?))[..12]
        ))
    }
}
pub fn architecture() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        _ => "unsupported",
    }
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn safe_path(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && !name.contains(['\\', '\n', '\r'])
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !name
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
}
fn allowed(name: &str) -> bool {
    matches!(
        name,
        "bin/aarch64/ungoggled"
            | "dji-hdmi.service"
            | "ungoggled-update-recovery.service"
            | "prepare-pi.sh"
            | "install.sh"
            | "RELEASE_NOTES.md"
            | "README.md"
            | "THIRD_PARTY.md"
    ) || name.starts_with("web/")
        // Offline-installable .deb files for an optional system dependency
        // (currently just mpv, for USB-drive video playback); see
        // `install::install_bundled_packages`.
        || (name.starts_with("packages/") && name.ends_with(".deb"))
}
pub fn unpack(archive: &Path, destination: &Path, arch: &str) -> Result<Manifest> {
    ensure!(
        fs::metadata(archive)?.len() <= MAX_UPLOAD as u64,
        "Update file exceeds 64 MiB"
    );
    fs::create_dir(destination)?;
    let result = (|| {
        let decoder = flate2::read::GzDecoder::new(fs::File::open(archive)?);
        let mut tar = tar::Archive::new(decoder);
        let mut total = 0_u64;
        let mut names = std::collections::HashSet::new();
        for entry in tar.entries()? {
            let mut entry = entry?;
            let name = std::str::from_utf8(&entry.path_bytes())?.to_owned();
            ensure!(safe_path(&name), "Unsafe archive path");
            ensure!(
                entry.header().entry_type().is_file(),
                "Only regular files are allowed in an update"
            );
            ensure!(
                name == "manifest.json" || allowed(&name),
                "Unexpected update file: {name}"
            );
            ensure!(
                names.insert(name.clone()) && names.len() <= 4096,
                "Duplicate files or too many entries"
            );
            let size = entry.size();
            total = total.checked_add(size).context("Archive size overflow")?;
            ensure!(total <= MAX_EXPANDED, "Expanded update exceeds 256 MiB");
            if name == "manifest.json" {
                ensure!(size <= 1024 * 1024, "Manifest too large");
            }
            let path = destination.join(name);
            fs::create_dir_all(path.parent().unwrap())?;
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            ensure!(
                std::io::copy(&mut entry, &mut out)? == size,
                "Truncated update file"
            );
            out.flush()?;
            out.sync_all()?;
        }
        // Read the compressed footer as well, so a corrupt gzip checksum is rejected.
        let mut decoder = tar.into_inner();
        let mut remaining = std::io::Read::by_ref(&mut decoder).take(1024 * 1024 + 1);
        let mut tail = Vec::new();
        remaining.read_to_end(&mut tail)?;
        ensure!(
            tail.len() <= 1024 * 1024 && tail.iter().all(|b| *b == 0),
            "Unexpected archive trailer"
        );
        validate(destination, arch)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}
pub fn validate(dir: &Path, arch: &str) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(dir.join("manifest.json"))?)
        .context("Not an ungoggled update file")?;
    ensure!(
        manifest.format == 1 && manifest.product == "ungoggled",
        "Unsupported update format or product"
    );
    let version = Version::parse(&manifest.version)?;
    ensure!(
        version.build.is_empty() && version.pre.is_empty(),
        "Use a stable release version"
    );
    ensure!(
        version >= Version::parse(env!("CARGO_PKG_VERSION"))?,
        "Downgrades are not supported"
    );
    ensure!(
        Version::parse(&manifest.minimum_updater)? <= Version::parse(env!("CARGO_PKG_VERSION"))?,
        "Install an intermediate updater version first"
    );
    ensure!(manifest.settings_schema > 0, "Missing settings schema");
    ensure!(
        arch == "aarch64",
        "This update requires 64-bit Raspberry Pi OS on a Pi 4"
    );
    for required in [
        format!("bin/{arch}/ungoggled"),
        "web/index.html".into(),
        "prepare-pi.sh".into(),
        "dji-hdmi.service".into(),
        "ungoggled-update-recovery.service".into(),
        "RELEASE_NOTES.md".into(),
        "install.sh".into(),
    ] {
        ensure!(
            manifest.files.contains_key(&required),
            "Missing required file: {required}"
        );
    }
    let mut actual = BTreeMap::new();
    inventory(dir, dir, &mut actual)?;
    ensure!(
        actual.len() == manifest.files.len(),
        "Update file inventory differs from manifest"
    );
    for (name, info) in &manifest.files {
        ensure!(safe_path(name) && allowed(name), "Invalid manifest path");
        let found = actual.get(name).context("Manifest file is missing")?;
        ensure!(
            found.size == info.size && found.sha256 == info.sha256,
            "Checksum mismatch: {name}"
        );
    }
    Ok(manifest)
}
fn inventory(root: &Path, dir: &Path, files: &mut BTreeMap<String, FileInfo>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            inventory(root, &entry.path(), files)?;
        } else if ty.is_file() {
            let name = entry
                .path()
                .strip_prefix(root)?
                .to_str()
                .context("Non-UTF8 path")?
                .to_owned();
            if name == "manifest.json" {
                continue;
            }
            let mut f = fs::File::open(entry.path())?;
            let mut h = Sha256::new();
            std::io::copy(&mut f, &mut h)?;
            files.insert(
                name,
                FileInfo {
                    size: entry.metadata()?.len(),
                    sha256: hex(&h.finalize()),
                },
            );
        } else {
            bail!("Links and special files are not allowed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::tests::{Fixture, bundle};
    fn archive(path: &Path, entries: &[(&str, &[u8], tar::EntryType)]) {
        let gzip = flate2::write::GzEncoder::new(
            fs::File::create(path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gzip);
        for (name, data, kind) in entries {
            let mut h = tar::Header::new_ustar();
            // Raw names allow traversal fixtures that tar::Header::set_path refuses.
            h.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            h.set_entry_type(*kind);
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append(&h, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }
    #[test]
    fn roundtrip_arm64_and_corruption() {
        let f = Fixture::new();
        let source = f.0.join("source");
        let m = bundle(&source);
        let mut names: Vec<_> = m.files.keys().cloned().collect();
        names.push("manifest.json".into());
        let contents: Vec<_> = names
            .iter()
            .map(|n| fs::read(source.join(n)).unwrap())
            .collect();
        let entries: Vec<_> = names
            .iter()
            .zip(&contents)
            .map(|(n, c)| (n.as_str(), c.as_slice(), tar::EntryType::Regular))
            .collect();
        let gz = f.0.join("update.gz");
        archive(&gz, &entries);
        let unpacked = unpack(&gz, &f.0.join("unpacked"), "aarch64").unwrap();
        assert_eq!(unpacked.release_id().unwrap(), m.release_id().unwrap());
        fs::write(source.join("web/index.html"), "damaged").unwrap();
        assert!(
            validate(&source, "aarch64")
                .unwrap_err()
                .to_string()
                .contains("Checksum")
        );
        let mut bytes = fs::read(&gz).unwrap();
        let i = bytes.len() - 8;
        bytes[i] ^= 1;
        fs::write(&gz, bytes).unwrap();
        assert!(unpack(&gz, &f.0.join("corrupt"), "aarch64").is_err());
        assert!(!f.0.join("corrupt").exists());
    }
    #[test]
    fn rejects_paths_links_and_duplicate_entries() {
        let f = Fixture::new();
        let gz = f.0.join("bad.gz");
        for name in [
            "../outside",
            "/absolute",
            "web/../../outside",
            "web//bad",
            "web/./bad",
            "web/bad\\name",
            "etc/shadow",
        ] {
            archive(&gz, &[(name, b"x", tar::EntryType::Regular)]);
            assert!(
                unpack(&gz, &f.0.join("unpacked"), "aarch64").is_err(),
                "{name}"
            );
        }
        for kind in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Directory,
            tar::EntryType::Char,
        ] {
            archive(&gz, &[("web/evil", b"", kind)]);
            assert!(unpack(&gz, &f.0.join("unpacked"), "aarch64").is_err());
        }
        archive(
            &gz,
            &[
                ("web/index.html", b"x", tar::EntryType::Regular),
                ("web/index.html", b"y", tar::EntryType::Regular),
            ],
        );
        assert!(
            unpack(&gz, &f.0.join("unpacked"), "aarch64")
                .unwrap_err()
                .to_string()
                .contains("Duplicate")
        );
        assert!(!f.0.join("outside").exists());
    }
    #[test]
    fn rejects_unsupported_versions_architectures_and_extra_files() {
        let f = Fixture::new();
        let mut m = bundle(&f.0);
        assert!(validate(&f.0, "x86_64").is_err());
        assert!(validate(&f.0, "armv7l").is_err());
        for (version, minimum) in [
            ("0.1.0", "0.3.0"),
            ("9.0.0-beta.1", "0.3.0"),
            ("9.0.0", "99.0.0"),
        ] {
            m.version = version.into();
            m.minimum_updater = minimum.into();
            fs::write(f.0.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
            assert!(validate(&f.0, "aarch64").is_err());
        }
        bundle(&f.0);
        fs::write(f.0.join("unlisted"), "extra").unwrap();
        assert!(validate(&f.0, "aarch64").is_err());
    }
    #[test]
    fn rejects_truncated_and_oversized_archives() {
        let f = Fixture::new();
        let gz = f.0.join("bad.gz");
        fs::File::create(&gz)
            .unwrap()
            .set_len(MAX_UPLOAD as u64 + 1)
            .unwrap();
        assert!(
            unpack(&gz, &f.0.join("out"), "aarch64")
                .unwrap_err()
                .to_string()
                .contains("64 MiB")
        );
        let mut h = tar::Header::new_ustar();
        h.set_path("web/huge").unwrap();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_size(MAX_EXPANDED + 1);
        h.set_mode(0o644);
        h.set_cksum();
        let mut zip = flate2::write::GzEncoder::new(
            fs::File::create(&gz).unwrap(),
            flate2::Compression::fast(),
        );
        zip.write_all(h.as_bytes()).unwrap();
        zip.finish().unwrap();
        assert!(
            unpack(&gz, &f.0.join("out"), "aarch64")
                .unwrap_err()
                .to_string()
                .contains("256 MiB")
        );
        fs::write(&gz, [0x1f, 0x8b, 0x08]).unwrap();
        assert!(unpack(&gz, &f.0.join("out"), "aarch64").is_err());
    }
}
