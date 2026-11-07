"""Mapping receipts bind to launch identity, never a reused synthetic PID."""
import hashlib
import importlib.util
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

SOURCE = Path(__file__).with_name('tools') / 'mappings.py'
SPEC = importlib.util.spec_from_file_location('synthetic_mappings', SOURCE)
mappings = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mappings)


class MappingIdentity(unittest.TestCase):
    def test_recorded_start_ticks_are_checked_before_reading_mappings(self):
        verify = Path(__file__).resolve().parents[3] / "build/verification"
        verify.mkdir(parents=True, exist_ok=True)
        with TemporaryDirectory(dir=verify) as directory:
            root = Path(directory)
            proc = root / 'proc/42'
            proc.mkdir(parents=True)
            executable = root / 'synthetic-elf'
            executable.write_bytes(b'synthetic executable marker')
            (proc / 'exe').symlink_to(executable)
            fields = ['S'] + ['0'] * 19
            fields[19] = '123'
            (proc / 'stat').write_text('42 (synthetic native) ' + ' '.join(fields))
            state = {'status':'running', 'cleared':True, 'pid':42, 'root_start_ticks':122,
                     'executable_sha256':hashlib.sha256(executable.read_bytes()).hexdigest()}
            (root / 'state.json').write_text(json.dumps(state))
            # smaps does not exist: a stale identity must fail before opening it.
            with self.assertRaisesRegex(ValueError, 'recorded launch'):
                mappings.collect(root, root / 'proc')
            state['root_start_ticks'] = 123
            (root / 'state.json').write_text(json.dumps(state))
            (proc / 'smaps').write_text(f'1000-2000 r-xp 00000000 00:00 1 {executable}\nRss: 4 kB\nPss: 3 kB\n')
            result = mappings.collect(root, root / 'proc')
            self.assertEqual(result['totals']['Pss'], 3)
            self.assertEqual(result['files']['<native executable>']['Rss'], 4)


if __name__ == '__main__':
    unittest.main()
