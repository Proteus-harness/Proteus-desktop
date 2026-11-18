"""Settings transfer must survive home changes without exporting account state."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('settings-migration.py')


class SettingsMigrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='proteus settings ')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / 'old home'
        self.target = self.root / 'new home'
        self.agent = self.source / '.config/Proteus-harness'
        self.desktop = self.source / '.config/dev.proteus.native'
        (self.agent / 'configs/prompts').mkdir(parents=True)
        (self.desktop / 'desktop').mkdir(parents=True)
        self.archive = self.root / 'settings.tar.gz'
        self.profile = self.agent / 'configs/config.toml'
        self.profile.write_text('prompt = ' + json.dumps(str(self.agent / 'configs/prompts/a.md')) + '\n')
        (self.agent / 'configs/prompts/a.md').write_text('Test instructions\n')
        (self.desktop / 'desktop/default.json').write_text('{"theme":"dark"}')
        self.preference = self.desktop / 'preferences.json'
        self.preference.write_text(json.dumps({
            'visual_configs': {'bindings': {str(self.profile): str(self.desktop / 'desktop/default.json')}},
            'recent': [str(self.source / 'project')],
            'sessions': {'private-session': {}},
        }))

    def run_script(self, operation, archive=None, home=None, *options):
        return subprocess.run(
            [sys.executable, str(SCRIPT), operation, str(archive or self.archive),
             '--home', str(home or self.source), *options],
            capture_output=True, text=True,
        )

    def test_round_trip_relocates_bindings_preserves_prompt_and_backs_up_existing_files(self):
        for relative in ('secrets/key.json', 'sessions/journal.jsonl', 'environment', 'configs/.env', 'model-selections/old.json'):
            path = self.agent / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('PRIVATE')
        self.assertEqual(self.run_script('export').returncode, 0)
        with tarfile.open(self.archive) as archive:
            names = archive.getnames()
            self.assertFalse(any('secrets' in name or 'sessions' in name or '.env' in name or 'model-selections' in name for name in names))
        preview = self.run_script('restore', self.archive, self.target, '--dry-run')
        self.assertEqual(preview.returncode, 0, preview.stderr)
        self.assertFalse(self.target.exists())
        target_profile = self.target / '.config/Proteus-harness/configs/config.toml'
        target_profile.parent.mkdir(parents=True)
        target_profile.write_text('model = "previous"\n')
        result = self.run_script('restore', home=self.target)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(str(self.target / '.config/Proteus-harness/configs/prompts/a.md'), target_profile.read_text())
        prefs = json.loads((self.target / '.config/dev.proteus.native/preferences.json').read_text())
        self.assertNotIn('sessions', prefs)
        self.assertNotIn('recent', prefs)
        self.assertIn(str(target_profile), prefs['visual_configs']['bindings'])
        saved = list((self.target / '.local/state/Proteus-settings-backups').glob('*/0'))
        self.assertEqual(saved[0].read_text(), 'model = "previous"\n')
        result = self.run_script('restore', home=self.target)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('Restored 0', result.stdout)

    def test_home_override_isolates_environment_and_explicit_root_takes_precedence(self):
        external = self.root / 'external agent'
        (external / 'configs').mkdir(parents=True)
        (external / 'configs/config.toml').write_text('model = "external"\n')
        with patch.dict(os.environ, {'PROTEUS_CONFIG_HOME': str(external), 'XDG_CONFIG_HOME': str(external)}):
            result = self.run_script('export')
            self.assertEqual(result.returncode, 0, result.stderr)
            with tarfile.open(self.archive) as archive:
                config = archive.extractfile('files/agent/configs/config.toml').read().decode()
                self.assertEqual(config, self.profile.read_text())
            explicit = self.root / 'explicit.tar.gz'
            result = self.run_script('export', explicit, self.source, '--agent-config', str(external))
            self.assertEqual(result.returncode, 0, result.stderr)
            with tarfile.open(explicit) as archive:
                self.assertEqual(archive.extractfile('files/agent/configs/config.toml').read(), b'model = "external"\n')

    def test_archive_paths_and_links_are_rejected_before_writing(self):
        for name, link in [('../outside', False), ('/tmp/outside', False), ('files/agent/configs/link', True)]:
            with self.subTest(name=name):
                archive_path = self.root / 'hostile.tar.gz'
                with tarfile.open(archive_path, 'w:gz') as archive:
                    entry = tarfile.TarInfo(name)
                    if link:
                        entry.type = tarfile.SYMTYPE
                        entry.linkname = '/etc/passwd'
                        archive.addfile(entry)
                    else:
                        entry.size = 1
                        archive.addfile(entry, io.BytesIO(b'x'))
                self.assertNotEqual(self.run_script('restore', archive_path, self.target).returncode, 0)
                self.assertFalse(self.target.exists())

    def test_export_refuses_linked_files_and_inline_credentials(self):
        link = self.agent / 'configs/linked'
        link.symlink_to(self.profile)
        result = self.run_script('export')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Symlink', result.stderr)
        link.unlink()
        self.profile.write_text('[provider]\napi_key = "PRIVATE"\n')
        result = self.run_script('export')
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('PRIVATE', result.stderr)
        self.assertFalse(self.archive.exists())


if __name__ == '__main__':
    unittest.main()
