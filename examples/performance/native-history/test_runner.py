"""Native Rust renderers and their Node worker belong to the extension total."""
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
import tomllib

from run import process_group, canonical_files, validate_canonical, profile, storage_binary, idle_cpu, idle_medians


class ProcessGroups(unittest.TestCase):
    def test_profile_selects_explicit_jsonl_storage_and_records_its_command(self):
        core = Path('/portable/bin/proteus')
        storage = storage_binary(core)
        self.assertEqual(storage, Path('/portable/bin/proteus-session-storage-module'))
        custom = Path('/custom path/storage-module')
        self.assertEqual(storage_binary(core, custom), custom)
        config = tomllib.loads(profile(Path('/portable/bin/proteus-reference-module'), custom))
        self.assertEqual(config['modules']['session_storage'], 'jsonl')
        component = config['components']['session-storage']
        self.assertEqual(component['command'], str(custom))
        self.assertEqual(component['exports']['session_storage'], {'jsonl': {}})

    def test_canonical_oracle_detects_added_deleted_and_modified_blobs(self):
        verify = Path(__file__).resolve().parents[3] / "build/verification"
        verify.mkdir(parents=True, exist_ok=True)
        with TemporaryDirectory(dir=verify) as directory:
            root = Path(directory)
            (root / 'journal.jsonl').write_text('synthetic journal\n')
            (root / 'blobs').mkdir()
            blob = root / 'blobs/first'
            blob.write_text('first')
            expected = canonical_files(root)
            validate_canonical(root, expected)
            (root / 'derived-ui-cache').write_text('rebuildable')
            validate_canonical(root, expected)
            (root / 'blobs/new').write_text('unexpected')
            with self.assertRaisesRegex(AssertionError, 'added='):
                validate_canonical(root, expected)
            (root / 'blobs/new').unlink()
            blob.unlink()
            with self.assertRaisesRegex(AssertionError, 'deleted='):
                validate_canonical(root, expected)
            blob.write_text('changed')
            with self.assertRaisesRegex(AssertionError, 'changed='):
                validate_canonical(root, expected)

    def test_rust_packages_and_workers_are_counted_as_extensions(self):
        root = Path('/portable/share/extensions')
        core = Path('/portable/bin/proteus')
        module = Path('/portable/bin/proteus-reference-module')
        storage = Path('/portable/bin/proteus-session-storage-module')

        def group(row):
            return process_group(row, root, core=core, module=module, storage=storage)

        for executable in (root / 'markdown-render/bin/markdown-render',
                           root / 'files/bin/files', Path('/usr/bin/node'),
                           Path('/usr/bin/python3')):
            self.assertEqual(group({'exe': str(executable)}), 'extensions')
        self.assertEqual(group({'exe': '/usr/bin/node-22', 'name': 'node'}), 'extensions')
        self.assertEqual(group({'exe': '/portable/share/extensions-old/bin/files'}), 'other')
        self.assertEqual(group({'exe': str(core)}), 'backend')
        self.assertEqual(group({'exe': str(module)}), 'modules')
        self.assertEqual(group({'exe': str(storage)}), 'modules')

    def test_exact_selected_paths_win_without_basename_or_argv_fallback(self):
        extensions = Path('/portable/share/extensions')
        core = Path('/alternate/core debug')
        module = Path('/alternate/reference debug')
        storage = extensions / 'nested/storage release'

        def group(row):
            return process_group(row, extensions, core=core, module=module, storage=storage)

        for exe, expected in ((core, 'backend'), (module, 'modules'), (storage, 'modules')):
            self.assertEqual(group({'exe': str(exe), 'argv': ['/unrelated/proteus']}), expected)
        for unknown in ('/unrelated/proteus', '/unrelated/proteus-reference-module',
                        '/unrelated/proteus-session-storage-module', str(core) + ' (deleted)',
                        str(storage.name), '', None):
            self.assertEqual(group({'exe': unknown, 'argv': [str(core)], 'name': 'proteus'}), 'other')
        self.assertEqual(group({'name': 'node'}), 'extensions')
        self.assertEqual(group({'exe': str(extensions / 'deep/old/proteus')}), 'extensions')


class IdleMemory(unittest.TestCase):
    def test_optional_breakdown_preserves_missing_and_zero(self):
        rows = [dict(pss_kib=10, pss_anon_kib=10, pss_shmem_kib=0),
                dict(pss_kib=20, pss_anon_kib=18, pss_file_kib=2, pss_shmem_kib=0)]
        self.assertEqual(idle_medians(rows, ('pss_kib',)),
                         dict(pss_kib=15, pss_anon_kib=14, pss_file_kib=None, pss_shmem_kib=0))


class IdleCpu(unittest.TestCase):
    def test_crossing_sample_is_not_idle_cpu_in_either_sampler(self):
        for modern in (False, True):
            with self.subTest(modern=modern):
                samples = [
                    dict(phase='settle'),
                    dict(phase='idle', interval_seconds=.30, interval_counters={'cpu_ticks': 30}),
                    dict(phase='idle', interval_seconds=.15, interval_counters={'cpu_ticks': 0}),
                ]
                if modern:
                    samples[1]['phase_interval_eligible'] = False
                    samples[2]['phase_interval_eligible'] = True
                self.assertEqual(idle_cpu(samples, 100), (0, 0, 1))

    def test_both_cpu_summaries_use_the_same_intervals(self):
        samples = [
            dict(phase='idle', interval_seconds=.25, interval_counters={'cpu_ticks': 25},
                 phase_interval_eligible=True),
            dict(phase='idle', interval_seconds=.75, interval_counters={'cpu_ticks': 0},
                 phase_interval_eligible=True),
            dict(phase='idle', interval_seconds=2, interval_counters={'cpu_ticks': 2000},
                 phase_interval_eligible=False),
            dict(phase='idle', interval_seconds=5, interval_counters={'cpu_ticks': 5000}),
        ]
        self.assertEqual(idle_cpu(samples, 100), (50, 25, 2))

    def test_absent_or_invalid_intervals_do_not_mean_zero_cpu(self):
        samples = [
            dict(phase='idle', interval_seconds=None, interval_counters=None),
            dict(phase='idle', interval_seconds=float('inf'), interval_counters={'cpu_ticks': 2}),
        ]
        self.assertEqual(idle_cpu(samples, 100), (None, None, 0))


if __name__ == '__main__':
    unittest.main()
