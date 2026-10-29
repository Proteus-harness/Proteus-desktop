#!/usr/bin/env python3
"""Repeatable synthetic native startup/RAM measurements inside a background display."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import statistics
import subprocess
import sys
import time
from urllib.parse import quote
from urllib.request import ProxyHandler, Request, build_opener


# Snapshot validation addresses this process's authenticated loopback backend.
DIRECT = build_opener(ProxyHandler({}))


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def profile(module):
    return f'''active_provider = "fake"
[providers.fake]
provider = "fake"
model = "fake-tool-model"
stream = true
[modules]
workflow = "coding.single_loop"
context = "simple"
policy = "allow_all"
hooks = []
[components.model]
command = {json.dumps(str(module))}
[components.model.exports.model.fake]
[components.workflow]
command = {json.dumps(str(module))}
[components.workflow.exports.workflow."coding.single_loop"]
[components.context]
command = {json.dumps(str(module))}
[components.context.exports.context.simple]
[components.capabilities]
command = {json.dumps(str(module))}
[components.capabilities.exports.policy.allow_all]
[components.capabilities.exports.tool."reference.tools"]
[module_config.model.fake]
implementation = "fake"
[tools]
enabled = ["read_file"]
[agent_control]
surface = "none"
roles = []
'''


def traces(path):
    result = {}
    windows = []
    for line in path.read_text(errors='replace').splitlines():
        for prefix, destination in [('PROTEUS_STARTUP ', result), ('PROTEUS_HISTORY ', windows)]:
            if prefix not in line:
                continue
            try:
                value = json.loads(line.split(prefix, 1)[1])
                if isinstance(destination, list):
                    destination.append(value)
                else:
                    destination.setdefault(value['stage'], value)
            except (ValueError, KeyError):
                pass
    return result, windows


def process_group(row, extensions):
    path = Path(row.get('exe', ''))
    executable = path.name
    if executable == 'proteus':
        return 'backend'
    if executable == 'proteus-reference-module':
        return 'modules'
    if path.is_relative_to(extensions) or executable.startswith('python') or executable == 'node' or row.get('name') == 'node':
        return 'extensions'
    return 'other'


def validate_snapshot(sample, session, expected):
    """Inspect the already-running backend after idle; never warm it before timing."""
    backend, = [row for row in sample['processes'] if row['group'] == 'backend']
    args = backend['argv']
    token = args[args.index('--token') + 1]
    proc = Path('/proc') / str(backend['pid'])
    sockets = set()
    for file in (proc / 'fd').iterdir():
        try:
            link = os.readlink(file)
            if link.startswith('socket:['):
                sockets.add(link[8:-1])
        except OSError:
            pass
    listeners = []
    for line in (proc / 'net/tcp').read_text().splitlines()[1:]:
        fields = line.split()
        if fields[3] == '0A' and fields[9] in sockets and fields[1].startswith('0100007F:'):
            listeners.append(int(fields[1].split(':')[1], 16))
    port, = listeners
    request = Request(f'http://127.0.0.1:{port}/events?session_dir='+quote(str(session), safe=''),
                      headers={'Authorization':'Bearer '+token})
    with DIRECT.open(request, timeout=30) as response:
        for line in response:
            if not line.startswith(b'data: '):
                continue
            event = json.loads(line[6:]).get('event', {})
            if event.get('type') != 'session_snapshot':
                continue
            snapshot = event['snapshot']
            assert snapshot['session_id'] == expected['session_id'], 'wrong resumed session'
            transcript = snapshot['transcript']
            before = (snapshot.get('history') or {}).get('before', 0)
            assert before + len(transcript) == expected['rows'], 'wrong actual message count'
            tail = [dict(message_id=row['message_id'], text=row['text']) for row in transcript[-50:]]
            assert tail == expected['tail'], 'wrong message IDs/text in resumed tail'
            return dict(session_id=snapshot['session_id'], messages=before+len(transcript),
                        visible_messages=len(transcript), history_before=before, tail_matched=True)
    raise RuntimeError('missing validation snapshot')


def terminate(process, tree):
    """Reap only the launched process identities, including escaped descendants."""
    from proc_sampler import table
    tree.sample('teardown')
    for force in (False, True):
        current = table()
        alive = [pid for pid, start in tree.owned.items()
                 if pid in current and current[pid]['start_ticks'] == start
                 and current[pid]['state'] != 'Z']
        for pid in reversed(alive):
            try:
                os.kill(pid, signal.SIGKILL if force else signal.SIGTERM)
            except ProcessLookupError:
                pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        time.sleep(.15)
    current = table()
    assert not any(pid in current and current[pid]['start_ticks'] == start
                   and current[pid]['state'] != 'Z' for pid, start in tree.owned.items()), 'surviving descendants'


def run_case(args, case, workspace, config, restart, expected):
    from proc_sampler import Tree, Recorder
    target = case / ('restart' if restart else 'first')
    target.mkdir()
    log = target / 'native.log'
    journal = next((case / 'config/sessions').rglob('journal.jsonl'))
    canonical = {str(path.relative_to(journal.parent)):sha(path) for path in journal.parent.rglob('*')
                 if path.is_file() and (path.name == 'journal.jsonl' or 'blobs' in path.relative_to(journal.parent).parts)}
    before = canonical['journal.jsonl']
    environment = os.environ.copy()
    environment.update(PROTEUS_BIN=str(args.core), PROTEUS_STARTUP_TRACE='1',
                       PROTEUS_EXTENSIONS_DIR=str(args.extensions),
                       PROTEUS_CONFIG_HOME=str(case / 'config'),
                       PROTEUS_HOME=str(case / 'data'), XDG_CONFIG_HOME=str(case / 'xdg-config'),
                       XDG_DATA_HOME=str(case / 'xdg-data'), XDG_CACHE_HOME=str(case / 'xdg-cache'))

    def classify(row):
        return process_group(row, args.extensions)

    samples = []
    started = time.monotonic()
    ready_at = None
    with log.open('w') as error:
        process = subprocess.Popen([str(args.native), str(workspace), '--config', str(config)],
                                   env=environment, stdout=error, stderr=error, start_new_session=True)
        try:
            tree = Tree(process.pid, classifier=classify, root_group='gui')
            save(args.output / 'active.json', dict(pid=process.pid, case=str(target), native_log=str(log)))
        except BaseException:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            raise
        with Recorder(tree, target / 'samples.jsonl') as recorder:
            recorder.mark('startup', external_spawn_monotonic=started)
            try:
                while True:
                    assert process.poll() is None, log.read_text(errors='replace')[-4000:]
                    stages, history = traces(log)
                    if ready_at is None and f'chat.transcript.{args.readiness}' in stages:
                        ready_at = time.monotonic()
                        recorder.mark('settle', external_readiness_observed_s=ready_at-started)
                    if ready_at is not None and time.monotonic()-ready_at >= args.settle:
                        if recorder.phase != 'idle':
                            recorder.mark('idle')
                    row = recorder.sample()
                    samples.append(row)
                    assert not row['memory_incomplete_pids'], row['memory_incomplete_pids']
                    if row['pss_swap_kib'] > args.max_pss_mib * 1024:
                        raise MemoryError('PSS+SwapPss safety threshold exceeded')
                    elapsed = time.monotonic()-started
                    if ready_at is not None and time.monotonic()-ready_at >= args.settle+args.idle:
                        break
                    if elapsed > args.timeout:
                        raise TimeoutError('native history readiness deadline')
                    time.sleep(args.interval)
                # Window inventory/capture are outside measured idle samples.
                validation = validate_snapshot(samples[-1], journal.parent, expected)
                save(target / 'snapshot-validation.json', validation)
                if args.bgui_session:
                    helper = [sys.executable, str(args.bgui)]
                    windows = subprocess.run(helper+['windows', args.bgui_session], capture_output=True, text=True, check=True)
                    (target / 'windows.json').write_text(windows.stdout)
                    subprocess.run(helper+['screenshot', args.bgui_session, '--path', str(target / 'window.png')],
                                   capture_output=True, check=True)
                    subprocess.run(helper+['key', args.bgui_session, 'alt+F4'], capture_output=True, check=True)
                    try:
                        process.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        pass
            finally:
                recorder.mark('teardown')
                terminate(process, tree)
    assert all(sha(journal.parent / path) == original for path, original in canonical.items()), 'read-only startup mutated canonical data'
    idle = [row for row in samples if row['phase'] == 'idle']
    assert len(idle) >= 2, 'insufficient steady samples'
    stages, history = traces(log)
    result = dict(status='passed', cache='restart' if restart else 'first-app-cache',
                  stages=stages, external_readiness_observed_s=ready_at-started,
                  median={key: statistics.median(row[key] for row in idle)
                          for key in ('rss_kib', 'pss_kib', 'swap_pss_kib', 'pss_swap_kib',
                                      'private_kib', 'interval_cpu_percent_one_core')},
                  groups={group: {key: statistics.median(row['groups'][group][key] for row in idle)
                                  for key in ('rss_kib', 'pss_kib', 'pss_swap_kib', 'process_count')}
                          for group in idle[-1]['groups']},
                  counters=tree.totals(), journal=dict(bytes=journal.stat().st_size, sha256=before, unchanged=True),
                  measured_peak_kib={key:max(row[key] for row in samples)
                                     for key in ('rss_kib','pss_kib','pss_swap_kib')},
                  idle_cpu_percent_one_core=100 * sum(row['interval_counters']['cpu_ticks'] for row in idle)
                  / tree.clock_ticks / sum(row['interval_seconds'] for row in idle),
                  startup_peak_kib={key:max((row[key] for row in samples if row['phase'] == 'startup'), default=None)
                                    for key in ('rss_kib','pss_kib','pss_swap_kib')},
                  history_windows=history, snapshot_validation=validation, canonical_files=canonical,
                  derived_cache_bytes=sum(p.stat().st_size for p in (case/'config').rglob('*')
                                          if p.is_file() and p.name != 'journal.jsonl'
                                          and 'blobs' not in p.relative_to(case/'config').parts))
    save(target / 'report.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('native', 'core', 'module', 'seeder', 'extensions', 'core-source', 'output'):
        parser.add_argument('--'+flag, type=Path, required=True)
    parser.add_argument('--messages', type=int, nargs='+', default=[5000, 50000])
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--text-repeats', type=int, default=4)
    parser.add_argument('--settle', type=float, default=5)
    parser.add_argument('--idle', type=float, default=5)
    parser.add_argument('--interval', type=float, default=.2)
    parser.add_argument('--readiness', choices=('layout', 'render'), default='layout',
                        help='require a positive visible row layout; render supports older baseline binaries')
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--max-pss-mib', type=int, default=4096)
    parser.add_argument('--bgui-session', required=True)
    parser.add_argument('--bgui', type=Path, required=True)
    args = parser.parse_args()
    for name in ('native', 'core', 'module', 'seeder', 'extensions', 'core_source', 'output', 'bgui'):
        setattr(args, name, getattr(args, name).resolve())
    assert args.repeats >= 1 and all(n > 0 for n in args.messages)
    assert args.interval >= .05 and args.idle >= 1 and args.settle >= 0
    assert os.environ.get('WAYLAND_DISPLAY'), 'launch through background-gui exec/run'
    sys.path.insert(0, str(args.core_source / 'examples/performance/common'))
    args.output.mkdir(parents=True, exist_ok=False)
    save(args.output / 'manifest.json', dict(
        binaries={name: dict(path=str(getattr(args,name)), sha256=sha(getattr(args,name)))
                  for name in ('native', 'core', 'module', 'seeder')},
        harness_sources={str(path):sha(path) for path in (
            Path(__file__).resolve(), args.core_source/'examples/performance/common/proc_sampler.py',
            args.core_source/'examples/performance/history-scale/verify.py',
            args.core_source/'examples/performance/history-scale/journal.py')},
        parameters={k:str(v) if isinstance(v,Path) else v for k,v in vars(args).items()},
        os_cache='warm/uncontrolled; no global cache dropping',
        clock='native stage elapsed from process.start; external observed readiness is polling upper bound',
        scope='native GUI and descendants; observer/compositor/sampler excluded; no model turns',
        pressure={p:Path('/proc/pressure/'+p).read_text() for p in ('cpu','memory','io')},
        meminfo=Path('/proc/meminfo').read_text()))
    workspace = args.output / 'workspace'
    workspace.mkdir()
    (workspace / 'AGENTS.md').write_text('Synthetic memory/startup benchmark. No model calls.\n')
    results = []
    for count in args.messages:
        for repeat in range(args.repeats):
            case = args.output / f'rows-{count}-repeat-{repeat}'
            configs = case / 'config/configs'
            configs.mkdir(parents=True)
            config = configs / 'bench.toml'
            config.write_text(profile(args.module))
            seed = subprocess.run([str(args.seeder), str(case/'config'), str(workspace),
                                   str(count), str(args.text_repeats)], capture_output=True, text=True, check=True)
            save(case / 'seed.json', json.loads(seed.stdout))
            subprocess.run([sys.executable, str(args.core_source/'examples/performance/history-scale/verify.py'),
                            'seed', json.loads(seed.stdout)['session_dir'], str(count), str(args.text_repeats),
                            str(case/'seed-verified.json')], capture_output=True, check=True)
            expected = json.loads((case/'seed-verified.json').read_text())
            expected.pop('ids')
            for restart in (False, True):
                result = run_case(args, case, workspace, config, restart, expected)
                result.update(messages=count, repeat=repeat)
                results.append(result)
                save(args.output / 'report.json', dict(status='running', results=results))
                print(json.dumps(dict(messages=count, repeat=repeat, cache=result['cache'],
                                      median=result['median'], stages=result['stages'])), flush=True)
    save(args.output / 'report.json', dict(status='passed', results=results))


if __name__ == '__main__':
    main()
