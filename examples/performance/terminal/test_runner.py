"""Failure-path process/receipt tests only; no terminal engine or GUI benchmark."""
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest

from run import ROOT, tree_memory

HERE = Path(__file__).resolve().parent
VERIFY = ROOT / 'build/verification'


class RunnerTests(unittest.TestCase):
    def setUp(self):
        VERIFY.mkdir(parents=True, exist_ok=True)
        self.temporary = TemporaryDirectory(prefix='terminal-runner-', dir=VERIFY)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_live_memory_read_failure_is_not_a_zero_complete_sample(self):
        proc = self.root / 'proc/42'
        (proc / 'task/42').mkdir(parents=True)
        fields = ['S'] + ['0'] * 19
        (proc / 'stat').write_text('42 (synthetic) ' + ' '.join(fields))
        (proc / 'status').write_text('VmRSS: 4 kB\n')
        # Missing smaps for a still-live identity is incomplete.
        self.assertEqual(tree_memory(42, self.root / 'proc')[3], [42])
        fields[0] = 'Z'
        (proc / 'stat').write_text('42 (synthetic) ' + ' '.join(fields))
        self.assertEqual(tree_memory(42, self.root / 'proc')[3], [])

    def test_cli_rejects_empty_runs_and_existing_receipts_before_start(self):
        report = self.root / 'report.json'
        report.write_text('original evidence')
        for arguments, message in [(['--runs', '0'], '--runs must be positive'),
                                   (['--runs', '-1'], '--runs must be positive'),
                                   ([], '--output already exists')]:
            result = subprocess.run([sys.executable, str(HERE / 'run.py'), '--output', str(report), *arguments],
                                    capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(message, result.stderr)
            self.assertEqual(report.read_text(), 'original evidence')

    def test_timeout_and_sampler_exception_reap_owned_separate_group_children(self):
        for failure in ('timeout', 'sampler'):
            with self.subTest(failure=failure):
                marker = self.root / f'{failure}.json'
                binary = self.root / f'fake-{failure}'
                binary.write_text('#!' + sys.executable + '\n' + f'''
import json, os, pathlib, subprocess, sys, time
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)
pathlib.Path({str(marker)!r}).write_text(json.dumps([os.getpid(), child.pid]))
time.sleep(30)
''')
                binary.chmod(0o700)
                bootstrap = f'''
import pathlib, time
import run
marker = pathlib.Path({str(marker)!r})
if {failure!r} == 'sampler':
    def fail_sample(pid):
        until = time.monotonic() + 3
        while not marker.exists() and time.monotonic() < until:
            time.sleep(.01)
        raise OSError('injected sample failure')
    run.tree_memory = fail_sample
run.run(pathlib.Path({str(binary)!r}), 'synthetic', 1, timeout=.2)
'''
                result = subprocess.run([sys.executable, '-c', bootstrap], cwd=HERE,
                                        capture_output=True, text=True, timeout=12)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('fixture timeout' if failure == 'timeout' else 'injected sample failure', result.stderr)
                self.assertTrue(marker.exists(), result.stderr)
                for pid in json.loads(marker.read_text()):
                    self.assertFalse(Path(f'/proc/{pid}').exists(), result.stderr)


if __name__ == '__main__':
    unittest.main()
