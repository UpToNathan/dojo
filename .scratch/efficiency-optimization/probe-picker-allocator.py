#!/usr/bin/env python3
"""Compare retained PSS after actual picker cycles in one owned nested compositor."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time


def identity(pid):
    process = Path('/proc') / str(pid)
    fields = (process / 'stat').read_text().rsplit(')', 1)[1].split()
    return {'pid': pid, 'start_ticks': int(fields[19]), 'executable': str((process / 'exe').resolve())}


def memory(process, expected, path):
    assert identity(process.pid) == expected, 'Kernel process identity changed'
    root = Path('/proc') / str(process.pid)
    smaps = (root / 'smaps').read_text()
    path.write_text(smaps)
    fields = dict(line.split(':', 1) for line in (root / 'smaps_rollup').read_text().splitlines() if ':' in line)
    io = dict(line.split(':', 1) for line in (root / 'io').read_text().splitlines() if ':' in line)
    return {'pss_KiB': int(fields['Pss'].split()[0]), 'rss_KiB': int(fields['Rss'].split()[0]), 'private_dirty_KiB': int(fields['Private_Dirty'].split()[0]), 'rchar': int(io['rchar']), 'time_ns': time.monotonic_ns()}


def command(runtime, message):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect(str(runtime / 'dojo.sock'))
        connection.sendall(message.encode() + b'\n')
        assert connection.recv(100).strip() == b'queued', f'{message} rejected'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    repo = args.repo.resolve()
    evidence = args.output or Path(tempfile.mkdtemp(prefix='dojo-picker-allocator-', dir='/tmp/dojo-optimization'))
    evidence.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, str(repo / 'tools/agent-input'))
    from run import Peer, screenshot, wait_for
    shell_binary = evidence / 'dojo-release'
    shutil.copy2(repo / 'target/release/dojo', shell_binary)
    compositor_binary = repo / 'vendor/niri/target/release/niri'
    source_image = repo / 'astro.jpg'
    baseline_env = dict(os.environ)
    session = dict(line.split('=', 1) for line in subprocess.check_output(['systemctl', '--user', 'show-environment'], text=True).splitlines() if '=' in line)
    host_runtime = Path(session.get('XDG_RUNTIME_DIR', baseline_env.get('XDG_RUNTIME_DIR', '')))
    host_wayland = Path(session.get('WAYLAND_DISPLAY', baseline_env.get('WAYLAND_DISPLAY', '')))
    if not host_wayland.is_absolute():
        host_wayland = host_runtime / host_wayland
    assert host_wayland.is_socket(), 'Host Wayland socket is unavailable'
    runtime = evidence / 'compositor-runtime'
    runtime.mkdir(mode=0o700)
    compositor_env = dict(baseline_env, XDG_RUNTIME_DIR=str(runtime), WAYLAND_DISPLAY=str(host_wayland), RUST_LOG='niri=info', DOJO_AGENT_INPUT='1')
    compositor_env.pop('NIRI_SOCKET', None)
    bus_config = evidence / 'session-bus.conf'
    bus_config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=' + str(runtime) + '</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>\n')
    dbus_log = open(evidence / 'dbus.log', 'w')
    bus = subprocess.Popen(['dbus-daemon', '--config-file=' + str(bus_config), '--nofork', '--print-address=1'], stdout=subprocess.PIPE, stderr=dbus_log)
    address = bus.stdout.readline().decode().strip()
    assert address.startswith('unix:'), 'Private bus did not start'
    compositor_env['DBUS_SESSION_BUS_ADDRESS'] = address
    compositor_log = open(evidence / 'niri.log', 'w')
    compositor = subprocess.Popen([str(compositor_binary), '--config', str(repo / 'tools/agent-input/nested.kdl')], env=compositor_env, stdout=compositor_log, stderr=compositor_log)
    query = None
    rows = []
    errors = []
    inherited = baseline_env.get('GLIBC_TUNABLES', '')
    metadata = {
        'claim': 'A fixed 128KiB mmap threshold reduces retained PSS after five actual picker cycles.',
        'primary_metric': 'closed PSS minus pre-picker PSS, KiB',
        'guard': 'command-to-Layers dialog mapping latency; polling resolution 20ms; no sleep-based latency claim',
        'fixed_bound_KiB': 64 * 1024,
        'post_warmup_drift_bound_KiB': 16 * 1024,
        'order': ['default', 'fixed'] * 5,
        'cycles_per_run': 5,
        'inherited_GLIBC_TUNABLES': inherited,
        'other_allocator_environment': {key: baseline_env[key] for key in ['MALLOC_MMAP_THRESHOLD_', 'MALLOC_TRIM_THRESHOLD_', 'MALLOC_ARENA_MAX'] if key in baseline_env},
        'shell_sha256': hashlib.sha256(shell_binary.read_bytes()).hexdigest(),
        'niri_sha256': hashlib.sha256(compositor_binary.read_bytes()).hexdigest(),
        'image_sha256': hashlib.sha256(source_image.read_bytes()).hexdigest(),
        'image_bytes': source_image.stat().st_size,
        'kernel': list(os.uname()),
        'boot_id': Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
        'logical_cpus': os.cpu_count(),
        'initial_load_average': os.getloadavg(),
        'evidence': str(evidence),
    }
    (evidence / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    print(json.dumps({'evidence': str(evidence), 'claim': metadata['claim']}), flush=True)
    try:
        def ready():
            assert compositor.poll() is None, 'Owned compositor exited during startup'
            log = (evidence / 'niri.log').read_text()
            display = re.search(r'listening on Wayland socket: (\S+)', log)
            ipc = re.search(r'IPC listening on: (\S+)', log)
            return (display.group(1), ipc.group(1)) if display and ipc else None
        display, ipc = wait_for(ready, 'owned nested compositor sockets')
        child_wayland = Path(display) if Path(display).is_absolute() else runtime / display
        assert child_wayland.is_socket() and Path(ipc).is_socket()
        query = Peer(ipc)
        metadata['compositor_identity'] = identity(compositor.pid)
        metadata['outputs'] = query.request('Outputs')['Outputs']
        for index, side in enumerate(metadata['order'], 1):
            run_directory = evidence / f'{index:02}-{side}'
            run_directory.mkdir()
            shell_runtime = run_directory / 'runtime'
            shell_runtime.mkdir(mode=0o700)
            shell_env = dict(baseline_env, XDG_RUNTIME_DIR=str(shell_runtime), WAYLAND_DISPLAY=str(child_wayland), NIRI_SOCKET=ipc, DBUS_SESSION_BUS_ADDRESS=address,
                             XDG_STATE_HOME=str(run_directory / 'state'), XDG_CONFIG_HOME=str(run_directory / 'config'), XDG_CACHE_HOME=str(run_directory / 'cache'))
            for key in ['XDG_STATE_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME']:
                Path(shell_env[key]).mkdir()
            if side == 'fixed':
                shell_env['GLIBC_TUNABLES'] = (inherited + ':' if inherited else '') + 'glibc.malloc.mmap_threshold=131072'
            row = {'run': index, 'side': side, 'GLIBC_TUNABLES': shell_env.get('GLIBC_TUNABLES', ''), 'load_average': os.getloadavg(), 'cycles': []}
            shell = None
            log = open(run_directory / 'shell.log', 'w')
            try:
                assert not any(layer['namespace'].startswith('dojo-') for layer in query.request('Layers')['Layers']), 'Previous shell layers remain'
                shell = subprocess.Popen([str(shell_binary), '--enable'], env=shell_env, stdout=log, stderr=log)
                expected = identity(shell.pid)
                row['identity'] = expected
                def layers():
                    assert shell.poll() is None, f'Shell exited with {shell.returncode}'
                    assert identity(shell.pid) == expected
                    return query.request('Layers')['Layers']
                wait_for(lambda: any(layer['namespace'].startswith('dojo-frame-') for layer in layers()), 'owned shell frame')
                time.sleep(1)
                row['before'] = memory(shell, expected, run_directory / 'before.smaps')
                for cycle in range(1, 6):
                    previous_io = memory(shell, expected, run_directory / f'{cycle}-pre-open.smaps')['rchar']
                    started = time.monotonic_ns()
                    command(shell_runtime, 'theme')
                    wait_for(lambda: any(layer['namespace'] == 'dojo-theme-picker' for layer in layers()), 'actual picker mapping')
                    mapped = time.monotonic_ns()
                    time.sleep(1.2)
                    opened = memory(shell, expected, run_directory / f'{cycle}-open.smaps')
                    assert opened['rchar'] - previous_io >= metadata['image_bytes'], 'Source wallpaper was not reread'
                    if cycle == 1:
                        screenshot(query, run_directory / 'preview.png')
                        row['preview_screenshot'] = str(run_directory / 'preview.png')
                        row['preview_sha256'] = hashlib.sha256((run_directory / 'preview.png').read_bytes()).hexdigest()
                    command(shell_runtime, 'theme')
                    wait_for(lambda: not any(layer['namespace'] == 'dojo-theme-picker' for layer in layers()), 'picker close')
                    time.sleep(0.5)
                    closed = memory(shell, expected, run_directory / f'{cycle}-closed.smaps')
                    row['cycles'].append({'cycle': cycle, 'mapping_ms': (mapped - started) / 1e6, 'opened': opened, 'closed': closed, 'source_read_delta': opened['rchar'] - previous_io})
                row['growth_KiB'] = row['cycles'][-1]['closed']['pss_KiB'] - row['before']['pss_KiB']
                row['post_warmup_drift_KiB'] = row['cycles'][-1]['closed']['pss_KiB'] - row['cycles'][0]['closed']['pss_KiB']
            except Exception as error:
                row['error'] = str(error)
                errors.append({'run': index, 'side': side, 'error': str(error)})
            finally:
                if shell is not None and shell.poll() is None:
                    command(shell_runtime, 'quit')
                    row['exit_code'] = shell.wait(timeout=8)
                    assert row['exit_code'] == 0, 'Owned shell failed during shutdown'
                log.close()
            rows.append(row)
            (run_directory / 'result.json').write_text(json.dumps(row, indent=2) + '\n')
            (evidence / 'progress.json').write_text(json.dumps({'metadata': metadata, 'runs': rows, 'errors': errors}, indent=2) + '\n')
            print(json.dumps({key: row[key] for key in ['run', 'side', 'growth_KiB', 'post_warmup_drift_KiB', 'error'] if key in row}), flush=True)
            assert not errors, f'Run failed; see {run_directory}'
        summary = {}
        for side in ['default', 'fixed']:
            matches = [row for row in rows if row['side'] == side]
            summary[side] = {}
            for metric in ['growth_KiB', 'post_warmup_drift_KiB']:
                values = [row[metric] for row in matches]
                summary[side][metric] = {'median': statistics.median(values), 'min': min(values), 'max': max(values)}
            latency = [cycle['mapping_ms'] for row in matches for cycle in row['cycles']]
            summary[side]['mapping_ms'] = {'median': statistics.median(latency), 'min': min(latency), 'max': max(latency), 'samples': len(latency)}
        result = {'metadata': metadata, 'runs': rows, 'summary': summary, 'errors': errors,
                  'preview_identical': len({row['preview_sha256'] for row in rows}) == 1,
                  'fixed_bounded': all(row['growth_KiB'] <= metadata['fixed_bound_KiB'] and row['post_warmup_drift_KiB'] <= metadata['post_warmup_drift_bound_KiB'] for row in rows if row['side'] == 'fixed')}
        (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'summary': summary, 'errors': len(errors), 'fixed_bounded': result['fixed_bounded'], 'evidence': str(evidence)}), flush=True)
        assert result['fixed_bounded'], 'Fixed allocator exceeded the predeclared retention budget'
        assert result['preview_identical'], 'Actual preview screenshots differed across runs'
    finally:
        if query is not None:
            query.close()
        compositor.terminate()
        compositor.wait(timeout=8)
        compositor_log.close()
        bus.terminate()
        bus.wait(timeout=8)
        dbus_log.close()


if __name__ == '__main__':
    main()
