#!/usr/bin/env python3
"""Remove standalone T3 autostart and enable the desktop backend for next login."""
import argparse
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess

SERVICE = r"app-t3\x2dserver@autostart.service"
HOME = Path.home()
APP = HOME / ".local/opt/t3code/AppRun"
SETTINGS = HOME / ".t3/userdata/desktop-settings.json"
AUTOSTART = HOME / ".config/autostart/t3-server.desktop"


def desktop_pids():
    result = []
    for path in Path('/proc').glob('[0-9]*/cmdline'):
        try:
            args = path.read_bytes().rstrip(b'\0').split(b'\0')
            if len(args) == 1 and args[0] == str(APP.parent / 't3code').encode():
                result.append(int(path.parent.name))
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    settings = json.loads(SETTINGS.read_text())
    if not isinstance(settings, dict):
        raise RuntimeError('Desktop settings must be a JSON object')
    if not APP.is_file():
        raise RuntimeError(f'Missing desktop launcher: {APP}')
    pids = desktop_pids()
    print(json.dumps({'desktop_pids': pids, 'service': SERVICE,
                      'autostart': str(AUTOSTART), 'local_enabled': settings.get('localEnvironmentEnabled')}), flush=True)
    if args.check:
        return
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    backup = HOME / '.local/state/dojo/t3-handoff' / stamp
    backup.mkdir(parents=True, exist_ok=False)
    shutil.copy2(SETTINGS, backup / SETTINGS.name)
    if AUTOSTART.exists() or AUTOSTART.is_symlink():
        if 't3-server-autostart' not in AUTOSTART.read_text():
            raise RuntimeError('Refusing to remove an unrelated autostart entry')
        AUTOSTART.rename(backup / AUTOSTART.name)
    # The running service also owns unrelated agent jobs. Leave it until logout.
    settings = json.loads(SETTINGS.read_text())
    settings['localEnvironmentEnabled'] = True
    temp = SETTINGS.with_name(f'.desktop-settings-handoff-{os.getpid()}.json')
    shutil.copy2(SETTINGS, temp)
    temp.write_text(json.dumps(settings, indent=2) + '\n')
    os.replace(temp, SETTINGS)
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
    if AUTOSTART.exists() or AUTOSTART.is_symlink():
        raise RuntimeError('Standalone autostart still exists')
    if not json.loads(SETTINGS.read_text()).get('localEnvironmentEnabled'):
        raise RuntimeError('Local backend setting was not saved')
    print(f'Standalone autostart removed; local backend enabled for next login. Backup: {backup}', flush=True)


if __name__ == '__main__':
    main()
