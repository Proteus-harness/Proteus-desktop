#!/usr/bin/env python3
"""Measure ordinary portable release builds without deleting incremental caches."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[3]


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def capture(command, cwd):
    return subprocess.run(command, cwd=cwd, check=True, capture_output=True, text=True).stdout


def revision(path):
    return {
        'root': str(path), 'head': capture(['git', 'rev-parse', 'HEAD'], path).strip(),
        'changes': capture(['git', 'status', '--porcelain'], path).splitlines(),
    }


def complexity(path):
    metadata = json.loads(capture(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
                                  '--filter-platform', 'x86_64-unknown-linux-gnu'], path))
    nodes = metadata['resolve']['nodes']
    members = set(metadata['workspace_members'])
    return {
        'workspace_packages': len(members), 'resolved_packages': len(nodes),
        'dependency_edges': sum(len(node['deps']) for node in nodes),
        'scope': 'Cargo resolve for x86_64 Linux, including dev/build dependencies; not a count of freshly compiled crates',
        'workspace_targets': {
            package['name']: [target['name'] for target in package['targets'] if 'bin' in target['kind']]
            for package in metadata['packages'] if package['id'] in members
        },
    }


def artifact(path):
    files = [item for item in path.rglob('*') if item.is_file() and not item.is_symlink()]
    return {'files': len(files), 'bytes': sum(item.stat().st_size for item in files),
            'binaries': {item.name: item.stat().st_size for item in (path / 'bin').iterdir() if item.is_file()}}


def measure(args, index, environment):
    from proc_sampler import Tree, Recorder
    target = args.output / f'run-{index:02}'
    target.mkdir()
    timing = target / 'time.json'
    command = ['/usr/bin/time', '-q', '-o', str(timing), '-f',
               '{"wall_seconds":%e,"user_seconds":%U,"system_seconds":%S,"max_process_rss_kib":%M,"major_faults":%F,"minor_faults":%R}',
               sys.executable, str(ROOT / 'scripts/portable.py'), '--no-archive', *args.portable_flags]
    started = time.monotonic()
    with (target / 'build.log').open('w') as log:
        process = subprocess.Popen(command, cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        tree = Tree(process.pid, root_group='other', classifier=lambda _: 'other')
        try:
            with Recorder(tree, target / 'samples.jsonl') as recorder:
                recorder.mark('portable-release', jobs=args.jobs)
                while process.poll() is None:
                    recorder.sample()
                    time.sleep(args.interval)
                recorder.sample()
        except BaseException:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            raise
    text = (target / 'build.log').read_text(errors='replace')
    result = {
        'exit_code': process.returncode, 'observed_wall_seconds': time.monotonic() - started,
        'time': json.loads(timing.read_text()) if timing.exists() else None,
        'sampled_tree': tree.totals(),
        'cargo_finished': [line.strip() for line in text.splitlines() if re.search(r'Finished `.*profile', line)],
        'compiled_packages': [line.strip() for line in text.splitlines() if line.lstrip().startswith('Compiling ')],
        'commands': [line for line in text.splitlines() if line.startswith('+ ')],
    }
    if process.returncode == 0:
        result['portable'] = artifact(ROOT / 'build/Proteus')
    save(target / 'report.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--core-source', type=Path, default=ROOT.parent / 'Proteus-core')
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--runs', type=int, default=2)
    parser.add_argument('--interval', type=float, default=.25)
    scope = parser.add_mutually_exclusive_group()
    scope.add_argument('--components', nargs='+', choices=('desktop', 'core', 'modules', 'extensions'))
    scope.add_argument('--full', action='store_true')
    parser.add_argument('--base', type=Path, help='ready portable runtime for partial builds')
    args = parser.parse_args()
    args.components = ['desktop', 'core', 'modules', 'extensions'] if args.full else (args.components or ['desktop'])
    args.portable_flags = ['--full'] if args.full else ['--components', *args.components]
    if args.base is not None:
        args.portable_flags += ['--base', str(args.base.resolve())]
    if args.jobs < 1 or args.runs < 1 or args.interval <= 0:
        parser.error('jobs/runs/interval must be positive')
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    args.core_source = args.core_source.resolve()
    sys.path.insert(0, str(args.core_source / 'examples/performance/common'))
    sys.path.insert(0, str(ROOT / 'scripts'))
    from sources import resolve
    keys = {'core': 'harness', 'modules': 'modules', 'extensions': 'extensions'}
    sources = {name: resolve(keys[name]) for name in args.components if name != 'desktop'}
    if 'desktop' in args.components:
        sources['desktop'] = ROOT
    environment = os.environ.copy()
    environment['CARGO_BUILD_JOBS'] = str(args.jobs)
    record = {
        'runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'profile': 'ordinary release, thin LTO, codegen-units=1; no archive',
        'components': args.components, 'portable_flags': args.portable_flags,
        'cache': 'existing Cargo/OS caches preserved; later runs have no intentional source changes',
        'jobs': args.jobs, 'interval_seconds': args.interval,
        'cargo_target_dir': environment.get('CARGO_TARGET_DIR'),
        'sources': {key: revision(path) for key, path in sources.items()}, 'runs': [],
        'measurement_scope': 'wall/CPU from GNU time; max_process_rss is one process, sampled PSS/RSS peaks sum the owned tree',
    }
    save(args.output / 'report.json', record)
    for index in range(1, args.runs + 1):
        result = measure(args, index, environment)
        record['runs'].append(result)
        save(args.output / 'report.json', record)
        if result['exit_code']:
            raise SystemExit(result['exit_code'])
        print(f"Run {index}: {result['time']['wall_seconds']} s, tree peak PSS {result['sampled_tree']['sampled_simultaneous_peaks_kib']['pss_kib']} KiB", flush=True)
    record['complexity'] = {key: complexity(path) for key, path in sources.items()}
    save(args.output / 'report.json', record)


if __name__ == '__main__':
    main()
