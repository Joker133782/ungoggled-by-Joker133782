#!/usr/bin/env python3
"""Assemble a full, ARM64 application update (format 1)."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent.parent
version = subprocess.check_output(['python3', 'scripts/version.py', '--check'], cwd=ROOT, text=True).strip()
files = {'bin/aarch64/ungoggled': (ROOT / 'target/aarch64-unknown-linux-musl/release/ungoggled').read_bytes()}
for p in sorted((ROOT / 'web/dist').rglob('*')):
    if p.is_file():
        files['web/' + p.relative_to(ROOT / 'web/dist').as_posix()] = p.read_bytes()
for name, source in {
    'prepare-pi.sh': 'scripts/prepare-pi.sh',
    'install.sh': 'scripts/install-update.sh',
    'dji-hdmi.service': 'deploy/dji-hdmi.service',
    'ungoggled-update-recovery.service': 'deploy/ungoggled-update-recovery.service',
    'README.md': 'README.md',
    'THIRD_PARTY.md': 'THIRD_PARTY.md',
}.items():
    files[name] = (ROOT / source).read_bytes()
# Lets an update offline-install mpv (for USB-drive video playback) on a Pi
# that predates it, without needing internet on the Pi. See scripts/bundle-mpv.sh.
bundled_packages = sorted((ROOT / 'build/mpv-debs').glob('*.deb'))
for p in bundled_packages:
    files['packages/' + p.name] = p.read_bytes()
files['RELEASE_NOTES.md'] = subprocess.check_output(['python3', 'scripts/version.py', '--notes'], cwd=ROOT)
# An installer older than 0.3.7 doesn't know the `packages/*.deb` manifest
# entry is allowed and rejects the whole file with a confusing "unexpected
# update file" error instead of the clear "install an intermediate version
# first" this field is meant to produce.
manifest = {
    'format': 1, 'product': 'ungoggled', 'version': version,
    'minimum_updater': '0.3.7' if bundled_packages else '0.3.0', 'settings_schema': 1,
    'files': {name: {'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()} for name, data in sorted(files.items())},
}
files['manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
assert sum(map(len, files.values())) < 256 * 1024 * 1024
out = ROOT / f'dist/ungoggled-{version}.update.tar.gz'
out.parent.mkdir(exist_ok=True)
# Only regular files, with no parent directory, timestamps, links, or PAX extensions.
with out.open('wb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', filename='', mtime=0) as zipped:
    with tarfile.open(fileobj=zipped, mode='w', format=tarfile.USTAR_FORMAT) as archive:
        for name, data in sorted(files.items()):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755 if name.startswith('bin/') or name.endswith('.sh') else 0o644
            archive.addfile(info, io.BytesIO(data))
assert out.stat().st_size < 64 * 1024 * 1024
out.with_name(out.name + '.sha256').write_text(hashlib.sha256(out.read_bytes()).hexdigest() + '  ' + out.name + '\n')
notes = ROOT / 'build/releases/RELEASE_NOTES.md'
notes.parent.mkdir(parents=True, exist_ok=True)
notes.write_bytes(files['RELEASE_NOTES.md'])
print(out)
