"""Native Rust renderers and their Node worker belong to the extension total."""
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
import tomllib

from run import process_group, canonical_files, validate_canonical, profile, storage_binary


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
        for executable in (root / 'markdown-render/bin/markdown-render',
                           root / 'files/bin/files', Path('/usr/bin/node'),
                           Path('/usr/bin/python3')):
            self.assertEqual(process_group({'exe': str(executable)}, root), 'extensions')
        self.assertEqual(process_group({'exe': '/usr/bin/node-22', 'name': 'node'}, root), 'extensions')
        self.assertEqual(process_group({'exe': '/portable/share/extensions-old/bin/files'}, root), 'other')
        self.assertEqual(process_group({'exe': '/portable/bin/proteus'}, root), 'backend')
        self.assertEqual(process_group({'exe': '/portable/bin/proteus-reference-module'}, root), 'modules')
        self.assertEqual(process_group({'exe': '/portable/bin/proteus-session-storage-module'}, root), 'modules')


if __name__ == '__main__':
    unittest.main()
