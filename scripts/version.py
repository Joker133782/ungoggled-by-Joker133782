#!/usr/bin/env python3
"""Cargo.toml is the release version; keep npm metadata and notes in sync."""
import argparse
import json
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--check', action='store_true')
parser.add_argument('--notes', action='store_true')
parser.add_argument('--set', metavar='X.Y.Z')
args = parser.parse_args()
cargo = ROOT / 'Cargo.toml'
if args.set:
    if not re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', args.set):
        parser.error('Use a stable semantic version: X.Y.Z')
    # `newline='\n'` on every write below: Path.write_text() otherwise
    # translates '\n' to the platform line separator, which silently
    # reintroduces CRLF into these normally-LF files when run from Windows
    # (confirmed root cause of a much bigger build mystery on 2026-10-31 —
    # see the "Resolved 2026-10-31" note in CLAUDE.md).
    cargo.write_text(re.sub(r'^version = "[^"]+"', f'version = "{args.set}"', cargo.read_text(), count=1, flags=re.M), newline='\n')
    for name in ['package.json', 'package-lock.json']:
        p = ROOT / 'web' / name
        data = json.loads(p.read_text())
        data['version'] = args.set
        if name == 'package-lock.json':
            data['packages']['']['version'] = args.set
        p.write_text(json.dumps(data, indent=2) + '\n', newline='\n')
    # Update only our own lock entry; dependency versions remain pinned.
    p = ROOT / 'Cargo.lock'
    p.write_text(re.sub(r'(name = "ungoggled"\nversion = ")[^"]+', lambda m: m[1] + args.set, p.read_text(), count=1), newline='\n')
version = tomllib.loads(cargo.read_text())['package']['version']
if args.check or args.notes:
    sections = re.split(r'^## ', (ROOT / 'CHANGELOG.md').read_text(), flags=re.M)
    notes = next((s for s in sections[1:] if s.splitlines()[0] == version), None)
    if not notes:
        parser.error(f'Add a ## {version} section to CHANGELOG.md')
    if args.check:
        for name in ['package.json', 'package-lock.json']:
            data = json.loads((ROOT / 'web' / name).read_text())
            assert data['version'] == version, f'{name}: version mismatch'
            if name == 'package-lock.json':
                assert data['packages']['']['version'] == version
        lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
        assert next(p['version'] for p in lock['package'] if p['name'] == 'ungoggled') == version
    if args.notes:
        print('## ' + notes.strip())
    else:
        print(version)
else:
    print(version)
