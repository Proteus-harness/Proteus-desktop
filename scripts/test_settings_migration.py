"""Settings transfer must survive home changes without exporting account state."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
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
        restored = tomllib.loads(target_profile.read_text())
        self.assertEqual(restored['modules']['session_storage'], 'jsonl')
        self.assertEqual(restored['components']['session-storage']['exports']['session_storage'], {'jsonl': {}})
        prefs = json.loads((self.target / '.config/dev.proteus.native/preferences.json').read_text())
        self.assertNotIn('sessions', prefs)
        self.assertNotIn('recent', prefs)
        self.assertIn(str(target_profile), prefs['visual_configs']['bindings'])
        saved = list((self.target / '.local/state/Proteus-settings-backups').glob('*/0'))
        self.assertEqual(saved[0].read_text(), 'model = "previous"\n')
        result = self.run_script('restore', home=self.target)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('Restored 0', result.stdout)

    def test_export_preferences_removes_window_state_and_preserves_portable_json(self):
        self.preference.write_text(
            '{"label":"старое","tabs":{"session":"inert"},"label":"Кириллица 🦋","pin":true}',
            encoding='utf-8',
        )
        result = self.run_script('export')
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(self.archive) as archive:
            member = archive.getmember('files/desktop/preferences.json')
            self.assertEqual(member.mode, 0o600)
            self.assertEqual(
                archive.extractfile(member).read(),
                '{\n  "label": "Кириллица 🦋",\n  "pin": true\n}\n'.encode(),
            )

    def test_export_rejects_bom_and_credentials_before_removing_window_state(self):
        for content, error in [
            (b'\xef\xbb\xbf{"tabs": {}}', 'Unexpected UTF-8 BOM'),
            (b'{"tabs":{"client-secret":"synthetic fixture"}}', 'Inline credential field'),
        ]:
            with self.subTest(error=error):
                self.preference.write_bytes(content)
                result = self.run_script('export')
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(error, result.stderr)
                self.assertFalse(self.archive.exists())

    def test_export_keeps_byte_decoder_for_a_preferences_directory_child(self):
        self.preference.unlink()
        self.preference.mkdir()
        (self.preference / 'serialized').write_bytes(
            '{"recent": ["inert"], "label": "Юникод"}'.encode('utf-16'),
        )
        result = self.run_script('export')
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(self.archive) as archive:
            self.assertEqual(
                archive.extractfile('files/desktop/preferences.json/serialized').read(),
                '{\n  "label": "Юникод"\n}\n'.encode(),
            )

    def test_restore_keeps_destination_window_state_and_replaces_portable_preferences(self):
        result = self.run_script('export')
        self.assertEqual(result.returncode, 0, result.stderr)
        destination = self.target / '.config/dev.proteus.native/preferences.json'
        destination.parent.mkdir(parents=True)
        # A local path can share the exporter's prefix; it must not be relocated.
        workspace = str(self.source / 'shared workspace')
        state = {
            'tabs': {'active': 'codex:local', 'roots': {'codex:local': {}}},
            'tabs_workspace': {'workspace': workspace, 'profile': None},
            'sessions': {'codex:local': {'title': 'Мой чат', 'pinned': True}},
            'recent': [workspace],
        }
        original = json.dumps({'visual_configs': {}, **state}).encode()
        destination.write_bytes(original)
        preview = self.run_script('restore', self.archive, self.target, '--dry-run')
        self.assertEqual(preview.returncode, 0, preview.stderr)
        self.assertEqual(destination.read_bytes(), original)
        result = self.run_script('restore', home=self.target)
        self.assertEqual(result.returncode, 0, result.stderr)
        restored = json.loads(destination.read_bytes())
        self.assertEqual({key: restored.get(key) for key in state}, state)
        profile = str(self.target / '.config/Proteus-harness/configs/config.toml')
        self.assertIn(profile, restored['visual_configs']['bindings'])
        repeat = self.run_script('restore', home=self.target)
        self.assertEqual(repeat.returncode, 0, repeat.stderr)
        self.assertIn('Restored 0', repeat.stdout)

    def test_invalid_local_preferences_abort_before_any_restore(self):
        self.assertEqual(self.run_script('export').returncode, 0)
        destination = self.target / '.config/dev.proteus.native/preferences.json'
        destination.parent.mkdir(parents=True)
        destination.write_bytes(b'null')
        result = self.run_script('restore', home=self.target)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('preferences', result.stderr)
        self.assertEqual(destination.read_bytes(), b'null')
        self.assertFalse((self.target / '.config/Proteus-harness').exists())

    def test_upgrade_storage_preserves_an_existing_included_storage_selection(self):
        shared = self.agent / 'configs/fragments/storage.toml'
        shared.parent.mkdir()
        shared.write_text('[modules]\nsession_storage = "sqlite"\n')
        modern = self.agent / 'configs/modern.config.toml'
        modern.write_text('include = ["fragments/storage.toml"]\n[modules]\ncontext = "simple"\n')
        legacy = 'modules = { context = "simple" }\n'
        self.profile.write_text(legacy)
        before = modern.read_bytes()
        command = [sys.executable, str(SCRIPT), 'upgrade-storage', '--home', str(self.source)]
        subprocess.run([*command, '--dry-run'], check=True, capture_output=True)
        self.assertEqual(self.profile.read_text(), legacy)
        result = subprocess.run(command, check=True, capture_output=True, text=True)
        self.assertIn('Updated 1', result.stdout)
        self.assertEqual(tomllib.loads(self.profile.read_text())['modules'],
                         {'context': 'simple', 'session_storage': 'jsonl'})
        self.assertEqual(modern.read_bytes(), before)

    def test_startup_migrates_only_the_stock_go_model_binding(self):
        import runpy
        convert = runpy.run_path(str(SCRIPT))['with_native_go']
        stock = ('[components.opencode-model]\ncommand = "proteus-opencode-module"\n'
                 '[components.opencode-model.exports.model.opencode]\n'
                 '[module_config.model.opencode]\nprovider = "opencode-go"\n'
                 'database_path = "/fixture/key-store"\n').encode()
        updated = convert(stock)
        profile = tomllib.loads(updated.decode())
        self.assertEqual(profile['components']['opencode-model']['command'], 'proteus-model-pack-module')
        self.assertEqual(profile['module_config']['model']['opencode'], {
            'implementation': 'opencode_go', 'provider': 'opencode-go', 'database_path': '/fixture/key-store',
        })
        self.assertEqual(convert(updated), updated)
        for custom in (stock + b'models_file = "/custom"\n', stock.replace(b'opencode-go', b'opencode-zen'),
                       b'include = ["parent.toml"]\n' + stock,
                       stock + b'[components.opencode-model.exports.workflow.opencode_loop]\n'):
            self.assertEqual(convert(custom), custom)

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
