#!/usr/bin/env python3
"""Record privileged configuration metadata, without copying file contents."""
import argparse
import json
import os
from pathlib import Path
import re
import stat
import subprocess

REPO = Path(__file__).resolve().parents[1]
PRIVATE = ('/etc/NetworkManager/system-connections', '/etc/ssh', '/etc/ssl/private',
           '/etc/cryptsetup-keys.d', '/etc/wireguard', '/etc/ipsec.d', '/etc/credstore',
           '/etc/credstore.encrypted', '/etc/1password', '/etc/openvpn', '/etc/iwd',
           '/etc/cups/ssl', '/etc/ly/save.txt')
PRIVATE_FILES = {'shadow', 'shadow-', 'gshadow', 'gshadow-', 'machine-id', 'hostname',
                 'hosts', 'passwd', 'passwd-', 'group', 'group-', 'fstab', 'crypttab',
                 'subuid', 'subgid', 'subuid-', 'subgid-', 'resolv.conf', 'adjtime'}
GENERATED = ('/etc/opengl-driver', '/etc/environment.d/99-nix-gl.conf', '/etc/ld.so.cache',
             '/etc/udev/hwdb.bin', '/etc/pacman.d/gnupg', '/etc/chromium/policies/dojo-backups',
             '/etc/ca-certificates/extracted', '/etc/ssl/certs', '/etc/.pwd.lock', '/etc/.updated',
             '/etc/chromium/policies/managed/dojo-theme.json', '/etc/resolv.pre-tailscale-backup.conf')


def beneath(path, prefix):
    return path == prefix or path.startswith(prefix + '/')


def exclusion(path):
    for prefix in PRIVATE:
        if beneath(path, prefix):
            return prefix, 'private-requires-separate-provisioning'
    name = Path(path).name
    if name in PRIVATE_FILES or any(token in name.lower() for token in ('secret', 'credential', 'token', '.key', '.pem', '.p12', '.pfx')):
        return str(Path(path).parent), 'private-requires-separate-provisioning'
    for prefix in GENERATED:
        if beneath(path, prefix):
            return prefix, 'generated-runtime'
    if '.backup-' in name or '.bak.' in name or name.endswith(('.pacsave', '.pacnew')):
        return str(Path(path).parent), 'local-backups-review-on-host'
    return None


def installer_sources(text):
    result = {}
    for array, kind in [('config_links', 'symlink'), ('config_copies', 'copy')]:
        match = re.search(r'\b' + array + r'=\((.*?)\n\)', text, re.S)
        if not match and array == 'config_links':
            continue
        if not match:
            raise ValueError(f'Cannot parse installer {array}; update inventory parser')
        for source, target, mode in re.findall(r"'([^'|]+)\|(/[^'|]+)(?:\|([0-7]+))?'", match[1]):
            result[target] = {'source': 'system-config/' + source, 'installation': kind}
            if mode:
                result[target]['expected_mode'] = mode
    return result


def pacman_metadata(query):
    owners = {}
    for line in query('-Ql').splitlines():
        package, _, path = line.partition(' ')
        if path:
            owners[path.rstrip('/')] = package
    backups = {}
    for line in query('-Qii').splitlines():
        match = re.search(r'(/\S+) \[(modified|unmodified|unreadable|missing)\]', line)
        if match:
            backups[match[1]] = match[2]
    return owners, backups


def capture(root, sources, owners, backups):
    entries = {}
    exclusions = set()
    gaps = set()

    def visit(path):
        excluded = exclusion(path)
        # Explicit maintained sources override broad private directory exclusion.
        if excluded and path not in sources:
            exclusions.add(excluded)
            return
        actual = root / path.lstrip('/')
        try:
            info = actual.lstat()
        except OSError:
            gaps.add(path)
            return
        if stat.S_ISDIR(info.st_mode):
            return
        owner = owners.get(path)
        if path not in sources and owner and backups.get(path) not in ('modified', 'unreadable', 'missing'):
            return
        entry = {'path': path, 'uid': info.st_uid, 'gid': info.st_gid,
                 'mode': f'{stat.S_IMODE(info.st_mode):04o}',
                 'type': 'symlink' if stat.S_ISLNK(info.st_mode) else 'file' if stat.S_ISREG(info.st_mode) else 'special',
                 'classification': 'repo-managed' if path in sources else 'package-backup' if owner else 'unowned-local'}
        if path in ('/etc/firewalld/firewalld.conf', '/etc/firewalld/zones/trusted.xml'):
            entry['setup_tool'] = 'system-config/bin/configure-firewalld'
            entry['setup_scope'] = 'default-zone=drop' if path.endswith('firewalld.conf') else 'trusted-interface=tailscale0'
        if owner:
            entry['package'] = owner
        if path in backups:
            entry['backup_status'] = backups[path]
        if path in sources:
            entry.update(sources[path])
            if stat.S_ISLNK(info.st_mode):
                expected = REPO / sources[path]['source']
                entry['source_link_matches'] = actual.resolve() == expected
        entries[path] = entry

    for prefix in ('/etc', '/usr/local'):
        base = root / prefix.lstrip('/')
        def error(exc):
            gaps.add('/' + str(Path(exc.filename).relative_to(root)))
        for directory, dirs, files in os.walk(base, followlinks=False, onerror=error):
            relative = '/' + str(Path(directory).relative_to(root))
            for name in list(dirs):
                path = relative + '/' + name
                excluded = exclusion(path)
                if excluded:
                    exclusions.add(excluded)
                    dirs.remove(name)
                    # Capture declared targets inside excluded directories only.
                    for target in sources:
                        if beneath(target, path):
                            visit(target)
                elif (Path(directory) / name).is_symlink():
                    visit(path)
                    dirs.remove(name)
            for name in files:
                visit(relative + '/' + name)
    for path in sources:
        if path not in entries:
            visit(path)
    for path, status in backups.items():
        if status == 'missing' and (beneath(path, '/etc') or beneath(path, '/usr/local')) and not exclusion(path):
            entries.setdefault(path, {'path': path, 'classification': 'package-backup', 'package': owners.get(path), 'backup_status': status})
    return {'schema_version': 1, 'scope': ['/etc', '/usr/local'],
            'files': [entries[key] for key in sorted(entries)],
            'excluded': [{'path': path, 'reason': reason} for path, reason in sorted(exclusions)],
            'inspection_gaps': sorted(gaps)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=REPO / 'system-config/inventory/privileged-files.json')
    parser.add_argument('--check', action='store_true', help='Exit 1 when metadata differs; do not write')
    args = parser.parse_args()
    def query(flag):
        return subprocess.run(['pacman', flag], check=True, capture_output=True, text=True, env={**os.environ, 'LC_ALL': 'C'}).stdout
    owners, backups = pacman_metadata(query)
    sources = installer_sources((REPO / 'scripts/install-system-config.sh').read_text())
    result = capture(Path('/'), sources, owners, backups)
    data = json.dumps(result, indent=2, sort_keys=True) + '\n'
    if args.check:
        if not args.output.exists() or args.output.read_text() != data:
            print('Privileged configuration metadata differs; rerun capture and review the diff.')
            return 1
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(data)
    print(f"Recorded {len(result['files'])} configuration paths; {len(result['inspection_gaps'])} inspection gaps.")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
