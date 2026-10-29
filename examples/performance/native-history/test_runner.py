"""Native Rust renderers and their Node worker belong to the extension total."""
from pathlib import Path
import unittest

from run import process_group


class ProcessGroups(unittest.TestCase):
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


if __name__ == '__main__':
    unittest.main()
