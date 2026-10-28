#!/usr/bin/env python3
"""Temporary release-profile research; no product settings are changed."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import time
import psutil

SAMPLE = 0.1
TIME_LIMIT = 3600
MIN_FREE = 2 * 1024**3
LINKERS = {'ld', 'ld64', 'ld.lld', 'lld', 'rust-lld', 'link', 'lld-link', 'collect2'}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--source', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--repeat', type=int, choices=[1, 2, 3])
    args = ap.parse_args()
    source, output = args.source.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip()
    metadata = {'source_sha': sha, 'platform': platform.platform(), 'machine': platform.machine(),
                'cpus': psutil.cpu_count(), 'memory_bytes': psutil.virtual_memory().total,
                'sample_seconds': SAMPLE, 'psutil': psutil.__version__}
    (output / 'machine.json').write_text(json.dumps(metadata, indent=2))
    profiles = [('thin-1', 'thin', '1'), ('thin-16', 'thin', '16'), ('off-1', 'off', '1')]
    repeats = [args.repeat] if args.repeat else [1, 2, 3]
    for repeat in repeats:
        ordered = profiles[repeat-1:] + profiles[:repeat-1]
        for label, lto, units in ordered:
            target = source / 'target'
            if target.exists():
                shutil.rmtree(target)
            run = output / f'{label}-{repeat}'
            run.mkdir(exist_ok=False)
            env = os.environ.copy()
            for key in list(env):
                if key.startswith(('CARGO_PROFILE_RELEASE_', 'HERDR_', 'HCOORD_')) or key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'):
                    del env[key]
            env.update(CARGO_PROFILE_RELEASE_LTO=lto, CARGO_PROFILE_RELEASE_CODEGEN_UNITS=units,
                       LC_ALL='C', LANG='C', HIDE_VERSION='0.0.0-profile-measure',
                       HIDE_COMMIT=sha, HIDE_BUILD_ORDER='0')
            command = ['bash', 'scripts/verify-cargo.sh', 'build', '--release', '--timings', '-p', 'hided', '--bins', '-p', 'hide-agent-hooks', '--bin', 'hide-agent-hooks']
            result = dict(metadata, profile=label, repeat=repeat, command=command,
                          clean_target=not target.exists(), free_before_bytes=shutil.disk_usage(source).free)
            if result['free_before_bytes'] < MIN_FREE:
                raise RuntimeError('Less than 2 GiB free before build')
            processes, linkers, defender = {}, {}, {}
            peak, link_active_seconds = 0, 0.0
            started = time.monotonic()
            last_sample = started
            with (run / 'build.log').open('w') as log:
                child = subprocess.Popen(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT,
                                         start_new_session=os.name != 'nt')
                root = psutil.Process(child.pid)
                try:
                    while child.poll() is None:
                        now = time.monotonic()
                        if now-started > TIME_LIMIT or shutil.disk_usage(source).free < MIN_FREE:
                            raise RuntimeError('Build exceeded one hour or 2 GiB free-disk floor')
                        rss, linking = 0, False
                        for proc in [root] + root.children(recursive=True):
                            try:
                                with proc.oneshot():
                                    name = proc.name()
                                    resident = proc.memory_info().rss
                                    identity = f'{proc.pid}:{proc.create_time()}'
                                    record = processes.setdefault(identity, {'pid': proc.pid, 'name': name, 'first_s': now-started, 'last_s': now-started, 'peak_rss_bytes': 0})
                                    record['last_s'] = now-started
                                    record['peak_rss_bytes'] = max(record['peak_rss_bytes'], resident)
                                    rss += resident
                                    if name.lower().removesuffix('.exe') in LINKERS:
                                        linking = True
                                        if identity not in linkers:
                                            linkers[identity] = dict(record, executable=proc.exe(), argv=proc.cmdline())
                                        linkers[identity].update(last_s=now-started, peak_rss_bytes=record['peak_rss_bytes'])
                            except (psutil.NoSuchProcess, psutil.AccessDenied):
                                continue
                        if linking:
                            link_active_seconds += now-last_sample
                        if os.name == 'nt':
                            for proc in psutil.process_iter(['name', 'pid', 'cpu_times', 'memory_info']):
                                if (proc.info['name'] or '').lower() == 'msmpeng.exe':
                                    cpu = proc.info['cpu_times']
                                    mem = proc.info['memory_info']
                                    rec = defender.setdefault(str(proc.pid), {'cpu_first': None if cpu is None else cpu.user+cpu.system, 'peak_rss_bytes': 0})
                                    rec['cpu_last'] = None if cpu is None else cpu.user+cpu.system
                                    if mem:
                                        rec['peak_rss_bytes'] = max(rec['peak_rss_bytes'], mem.rss)
                        peak = max(peak, rss)
                        last_sample = now
                        time.sleep(SAMPLE)
                    result.update(exit_code=child.wait(), total_seconds=time.monotonic()-started)
                finally:
                    if child.poll() is None:
                        if os.name == 'nt':
                            subprocess.run(['taskkill', '/PID', str(child.pid), '/T', '/F'], check=False)
                        else:
                            os.killpg(child.pid, signal.SIGTERM)
                        try:
                            child.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            if os.name != 'nt':
                                os.killpg(child.pid, signal.SIGKILL)
                            child.kill()
                            child.wait()
            result.update(peak_tree_rss_bytes=peak, sampled_link_active_seconds=link_active_seconds,
                          linker_observations=list(linkers.values()), defender=defender,
                          free_after_bytes=shutil.disk_usage(source).free)
            sizes = {}
            for binary in ('hided', 'hide', 'hide-agent-hooks'):
                path = target / 'release' / (binary + ('.exe' if os.name == 'nt' else ''))
                if path.exists():
                    sizes[binary] = {'bytes': path.stat().st_size, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
            result['binaries'] = sizes
            timings = target / 'cargo-timings'
            if timings.exists():
                shutil.copytree(timings, run / 'cargo-timings')
            (run / 'processes.json').write_text(json.dumps(list(processes.values()), indent=2))
            (run / 'result.json').write_text(json.dumps(result, indent=2))
            print(json.dumps({key: result[key] for key in ('profile', 'repeat', 'exit_code', 'total_seconds', 'peak_tree_rss_bytes', 'sampled_link_active_seconds', 'binaries')}), flush=True)
            shutil.rmtree(target)
            if result['exit_code']:
                raise RuntimeError(f'Build failed: {label} repeat {repeat}')


if __name__ == '__main__':
    main()
