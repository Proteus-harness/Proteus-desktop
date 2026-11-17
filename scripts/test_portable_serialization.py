"""Synthetic portable transactions: plain local files, no builders or executables."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import fcntl
import gzip
import importlib.util
import json
from pathlib import Path
import subprocess
import stat
import sys
import tarfile
import tempfile
import threading
import types
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent


def forbidden(*args, **kwargs):
    raise AssertionError('Unexpected build/source operation')


def module(name, **attributes):
    value = types.ModuleType(name)
    value.__dict__.update(attributes)
    return value


def load_source(name, filename):
    path = HERE / filename
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: value}):
        spec.loader.exec_module(value)
    if Path(value.__file__).resolve() != path:
        raise AssertionError('Wrong source module')
    return value


def load_portable():
    state = load_source('_serialization_state', 'portable_state.py')
    # Scope fake imports to loading only, retaining any pre-existing modules.
    replacements = {
        'sources': module('sources', resolve=forbidden),
        'limits': module('limits', limit=forbidden),
        'cargo_binary': module('cargo_binary', build_binary=forbidden),
        'desktop_toolchain': module('desktop_toolchain', prepare_desktop=forbidden),
        'portable_modules': module('portable_modules', assemble=forbidden),
        'portable_state': state,
        'release_metadata': module('release_metadata',
            install_component_notices=forbidden, source_record=forbidden,
            version=forbidden, write_manifest=forbidden),
    }
    with patch.dict(sys.modules, replacements):
        portable = load_source('_serialization_portable', 'portable.py')
    return portable, state


def read_fixture(base):
    return json.loads((base / 'share/release-sources.json').read_text())


def seed(base, value):
    (base / 'share').mkdir(parents=True)
    (base / 'share/release-sources.json').write_text(json.dumps({'version': 'fixture'}))
    for component in ('desktop', 'core', 'modules', 'extensions'):
        (base / f'{component}.txt').write_text(value)


def assemble_fixture(stage, selected, roots, profile, record=None):
    (stage / 'share').mkdir(exist_ok=True)
    (stage / 'share/release-sources.json').write_text(json.dumps({'version': 'fixture'}))
    for component in selected:
        (stage / f'{component}.txt').write_text('new')


def configure(root):
    portable, state = load_portable()
    portable.NATIVE = root
    portable.BUILD = root / 'build'
    portable.PORTABLE = portable.BUILD / 'Proteus'
    portable.read_record = read_fixture
    portable.resolve = lambda name: root / 'unused-source' / name
    portable.verify_protocol = lambda *args: None
    portable.verify_extension_source = lambda *args: None
    portable.verify_packaged_extensions = lambda *args: None
    portable.limit = lambda: None
    portable.assemble = assemble_fixture
    return portable, state


def lock_path(destination):
    return destination.parent / f'.{destination.name}.update.lock'


def is_locked(destination):
    with lock_path(destination).open('a') as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
        return False


class PortableSerializationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='portable-serialization-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_overlapping_default_base_updates_preserve_both_components(self):
        first, _ = configure(self.root)
        second, _ = configure(self.root)
        seed(first.PORTABLE, 'old')
        first_staged = threading.Event()
        release_first = threading.Event()
        second_attempt = threading.Event()

        def hold_first(*args):
            assemble_fixture(*args)
            first_staged.set()
            if not release_first.wait(10):
                raise AssertionError('Fixture barrier timed out')

        first.assemble = hold_first
        original_read = second.read_record

        def signal_read(base):
            second_attempt.set()
            return original_read(base)

        second.read_record = signal_read
        # Candidate announces its attempt before the real blocking lock;
        # unchanged baseline announces it at its ordinary initial base read.
        if hasattr(second, 'update_lock'):
            original_lock = second.update_lock

            @contextmanager
            def signal_lock(destination):
                second_attempt.set()
                with original_lock(destination):
                    yield

            second.update_lock = signal_lock

        with ThreadPoolExecutor(max_workers=2) as pool:
            a = pool.submit(first.main, ['--components', 'core', '--no-archive'])
            try:
                self.assertTrue(first_staged.wait(5), 'first stage not reached')
                held = is_locked(first.PORTABLE)
                b = pool.submit(second.main, ['--components', 'extensions', '--no-archive'])
                self.assertTrue(second_attempt.wait(5), 'second invocation did not start')
                if not held:
                    # Deterministic old behavior: publish the second complete
                    # update before allowing the first stale stage to publish.
                    b.result(timeout=5)
            finally:
                release_first.set()
            a.result(timeout=5)
            b.result(timeout=5)
        self.assertEqual((first.PORTABLE / 'core.txt').read_text(), 'new')
        self.assertEqual((first.PORTABLE / 'extensions.txt').read_text(), 'new')
        self.assertEqual((first.PORTABLE / 'modules.txt').read_text(), 'old')

    def test_lock_covers_snapshot_assembly_publication_and_archive(self):
        portable, _ = configure(self.root)
        seed(portable.PORTABLE, 'old')
        calls = []
        for name in ('read_record', 'assemble', 'verify_packaged_extensions', 'publish', 'archive'):
            original = getattr(portable, name)

            def checked(*args, _name=name, _original=original, **kwargs):
                self.assertTrue(is_locked(portable.PORTABLE), _name)
                calls.append(_name)
                return _original(*args, **kwargs)

            setattr(portable, name, checked)
        portable.main(['--components', 'core'])
        self.assertEqual(calls, ['read_record', 'assemble', 'read_record', 'verify_packaged_extensions', 'publish', 'archive'])
        archive = next(portable.BUILD.glob('*.tar.gz'))
        with tarfile.open(archive) as saved:
            self.assertEqual(saved.extractfile('Proteus/core.txt').read(), b'new')
            self.assertEqual(saved.extractfile('Proteus/extensions.txt').read(), b'old')
        self.assertFalse(is_locked(portable.PORTABLE))

    def test_archive_failure_preserves_previous_and_cleans_temporary(self):
        portable, _ = configure(self.root)
        seed(portable.PORTABLE, 'old')
        archive = portable.archive()
        archive.chmod(0o640)
        old_bytes = archive.read_bytes()
        (portable.PORTABLE / 'core.txt').write_text('new')
        for operation in ('add', 'compress', 'close', 'replace'):
            with self.subTest(operation=operation):
                target, name = {
                    'add': (tarfile.TarFile, 'add'),
                    'compress': (gzip.GzipFile, 'write'),
                    'close': (gzip.GzipFile, 'close'),
                    'replace': (portable.os, 'replace'),
                }[operation]
                original_close = gzip.GzipFile.close

                def fail_close(stream):
                    was_closed = stream.closed
                    original_close(stream)
                    if not was_closed:
                        raise OSError('synthetic archive failure')

                failure = (patch.object(target, name, fail_close) if operation == 'close'
                           else patch.object(target, name, side_effect=OSError('synthetic archive failure')))
                with failure:
                    with self.assertRaisesRegex(OSError, 'synthetic archive failure'):
                        portable.archive()
                self.assertEqual(archive.read_bytes(), old_bytes)
                self.assertEqual(list(portable.BUILD.glob(f'.{archive.name}.*')), [])
        self.assertEqual(portable.archive(), archive)
        self.assertEqual(stat.S_IMODE(archive.stat().st_mode), 0o640)
        with tarfile.open(archive) as saved:
            self.assertEqual(saved.extractfile('Proteus/core.txt').read(), b'new')
        self.assertEqual(list(portable.BUILD.glob(f'.{archive.name}.*')), [])

    def test_new_archive_keeps_normal_creation_permissions(self):
        portable, _ = configure(self.root)
        seed(portable.PORTABLE, 'old')
        reference = portable.BUILD / 'ordinary-created-file'
        reference.write_bytes(b'')
        archive = portable.archive()
        self.assertEqual(stat.S_IMODE(archive.stat().st_mode),
                         stat.S_IMODE(reference.stat().st_mode))

    def test_plan_does_not_create_build_or_lock(self):
        portable, _ = configure(self.root)
        portable.read_record = forbidden
        portable.main(['--components', 'core', '--plan'])
        self.assertFalse(portable.BUILD.exists())

    def test_build_failure_releases_lock_and_preserves_base(self):
        portable, _ = configure(self.root)
        seed(portable.PORTABLE, 'old')

        def fail(*args):
            self.assertTrue(is_locked(portable.PORTABLE))
            raise RuntimeError('synthetic assembly failure')

        portable.assemble = fail
        with self.assertRaisesRegex(RuntimeError, 'synthetic assembly failure'):
            portable.main(['--components', 'core', '--no-archive'])
        self.assertEqual((portable.PORTABLE / 'core.txt').read_text(), 'old')
        self.assertEqual(list(portable.BUILD.glob('Proteus.next-*')), [])
        self.assertTrue(lock_path(portable.PORTABLE).exists())
        self.assertFalse(is_locked(portable.PORTABLE))

    def test_full_and_explicit_reuse_share_destination_transaction(self):
        for mode in ('full', 'reuse'):
            with self.subTest(mode=mode):
                root = self.root / mode
                root.mkdir()
                portable, _ = configure(root)
                external = root / 'external'
                seed(external, 'external')
                original_publish = portable.publish

                def checked_publish(stage, destination):
                    self.assertTrue(is_locked(destination))
                    original_publish(stage, destination)

                portable.publish = checked_publish
                args = ['--full'] if mode == 'full' else ['--reuse-binaries', '--base', str(external)]
                portable.main([*args, '--no-archive'])
                expected = 'new' if mode == 'full' else 'external'
                self.assertEqual((portable.PORTABLE / 'core.txt').read_text(), expected)
                self.assertEqual((external / 'core.txt').read_text(), 'external')
                self.assertFalse(is_locked(portable.PORTABLE))

    def test_lock_excludes_another_process_and_keeps_same_inode(self):
        portable, state = configure(self.root)
        code = (
            'import fcntl,sys\n'
            'with open(sys.argv[1], "a") as f:\n'
            ' try: fcntl.flock(f, fcntl.LOCK_EX|fcntl.LOCK_NB)\n'
            ' except BlockingIOError: sys.exit(23)\n'
        )

        def probe():
            return subprocess.run(
                [sys.executable, '-I', '-B', '-S', '-c', code, str(lock_path(portable.PORTABLE))],
                cwd=self.root, capture_output=True, text=True, timeout=5,
            )

        with state.update_lock(portable.PORTABLE):
            inode = lock_path(portable.PORTABLE).stat().st_ino
            result = probe()
            self.assertEqual(result.returncode, 23, result.stderr)
        result = probe()
        self.assertEqual(result.returncode, 0, result.stderr)
        with state.update_lock(portable.PORTABLE):
            self.assertEqual(lock_path(portable.PORTABLE).stat().st_ino, inode)


if __name__ == '__main__':
    unittest.main()
