#!/usr/bin/env python3
"""Export Proteus settings and restore them on another machine (Python 3.11+)."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path, PurePosixPath
import re
import sys
import tomllib

SCHEMA = 1
MAX_FILE = 16 * 1024 * 1024
MAX_TOTAL = 128 * 1024 * 1024
SECTIONS = {
    'agent': ('configs', 'opencode-go/configs'),
    'desktop': ('desktop', 'preferences.json', 'extension-preferences'),
}
SECRET_FIELDS = {'api_key', 'access_token', 'refresh_token', 'password', 'client_secret', 'authorization', 'token'}
EXCLUDED = {'secrets', 'sessions', '.proteus', '.env', 'environment', '__pycache__', 'cache'}
WINDOW_STATE = {'tabs', 'tabs_workspace', 'sessions', 'recent'}
TOML_STRING = re.compile(r'# [^\n]*|#[^\n]*|"""[\s\S]*?"""|\x27\x27\x27[\s\S]*?\x27\x27\x27|"(?:\\.|[^"\\])*"|\x27[^\x27\n]*\x27')
STORAGE_COMPONENT = '''
[components.session-storage]
command = "proteus-session-storage-module"
transport.max_frame_bytes = 134217728

[components.session-storage.exports.session_storage.jsonl]
'''


def roots(args):
    home = args.home.expanduser().absolute() if args.home else Path.home()
    config = Path(os.environ.get('XDG_CONFIG_HOME', home / '.config')) if not args.home else home / '.config'
    agent = config / 'Proteus-harness' if args.home else Path(os.environ.get('PROTEUS_CONFIG_HOME', config / 'Proteus-harness'))
    return home, {
        'agent': (args.agent_config or agent).expanduser().absolute(),
        'desktop': (args.desktop_config or config / 'dev.proteus.native').expanduser().absolute(),
    }


def settings_state_dir(args, home):
    default = home / '.local/state'
    return Path(os.environ.get('XDG_STATE_HOME', default)) if not args.home else default


def regular_path(path):
    """Refuse linked ancestors as well as linked files at either end."""
    for part in [path, *path.parents]:
        if part.is_symlink():
            raise ValueError(f'Symlink is not supported: {part}')
    if path.exists() and not path.is_file():
        raise ValueError(f'Expected a regular file: {path}')


def allowed(name):
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or str(path) != name or len(path.parts) < 3:
        return False
    if path.parts[0] != 'files' or path.parts[1] not in SECTIONS:
        return False
    if EXCLUDED.intersection(path.parts[2:]):
        return False
    relative = '/'.join(path.parts[2:])
    return any(relative == item or relative.startswith(item + '/') for item in SECTIONS[path.parts[1]])


def structured(data, suffix):
    text = data.decode('utf-8')
    return json.loads(text) if suffix == '.json' else tomllib.loads(text)


def reject_credentials(value, location):
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = key.casefold().replace('-', '_')
            secret = normalized in SECRET_FIELDS or normalized.endswith(('_api_key', '_access_token', '_refresh_token', '_password', '_client_secret'))
            if secret and child not in (None, '', False):
                raise ValueError(f'Inline credential field in {location}; use a credential file or environment reference')
            reject_credentials(child, location)
    elif isinstance(value, list):
        for child in value:
            reject_credentials(child, location)


def read_settings(path):
    regular_path(path)
    if path.stat().st_size > MAX_FILE:
        raise ValueError(f'Settings file is too large: {path}')
    return path.read_bytes()


def _portable_preferences(preferences):
    for key in WINDOW_STATE:
        preferences.pop(key, None)
    return (json.dumps(preferences, ensure_ascii=False, indent=2) + '\n').encode()


def _export_data(path, preferences_source):
    data = read_settings(path)
    suffix = path.suffix
    if suffix in ('.json', '.toml'):
        decoded = structured(data, suffix)
        reject_credentials(decoded, path)
        if suffix == '.json' and path == preferences_source:
            return _portable_preferences(decoded)
        del decoded
    # If preferences.json is a directory, retain the old bytes decoder for its files.
    if preferences_source is not None:
        return _portable_preferences(json.loads(data))
    return data


def export_archive(args):
    import hashlib
    import io
    import tarfile
    import tempfile

    home, locations = roots(args)
    files = {}
    for kind, items in SECTIONS.items():
        for item in items:
            source = locations[kind] / item
            if source.is_symlink():
                raise ValueError(f'Symlink is not supported: {source}')
            if not source.exists():
                continue
            candidates = sorted(source.rglob('*')) if source.is_dir() else [source]
            preferences_source = source if kind == 'desktop' and item == 'preferences.json' else None
            for path in candidates:
                if path.is_symlink():
                    raise ValueError(f'Symlink is not supported: {path}')
                relative = path.relative_to(locations[kind])
                if path.is_dir() or EXCLUDED.intersection(relative.parts):
                    continue
                name = f'files/{kind}/{path.relative_to(locations[kind]).as_posix()}'
                files[name] = _export_data(path, preferences_source)
    if not files:
        raise ValueError('No Proteus settings found')
    if sum(map(len, files.values())) > MAX_TOTAL:
        raise ValueError('Settings exceed the archive size limit')
    manifest = {
        'schema': SCHEMA,
        'home': str(home),
        'roots': {key: str(value) for key, value in locations.items()},
        'files': {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    destination = args.archive.expanduser().absolute()
    regular_path(destination)
    if destination.exists():
        raise ValueError(f'Archive already exists: {destination}')
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix='.settings-', dir=destination.parent)
    os.close(descriptor)
    try:
        with tarfile.open(temporary, 'w:gz') as archive:
            for name, data in [('manifest.json', (json.dumps(manifest, indent=2) + '\n').encode()), *files.items()]:
                entry = tarfile.TarInfo(name)
                entry.mode = 0o600
                entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
        os.link(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)
    print(f'Exported {len(files)} settings files to {destination}')


def read_archive(path):
    import hashlib
    import tarfile

    files = {}
    total = 0
    with tarfile.open(path, 'r:gz') as archive:
        for member in archive:
            if not member.isfile() or member.name in files or member.size > MAX_FILE:
                raise ValueError(f'Invalid archive member: {member.name}')
            if member.name != 'manifest.json' and not allowed(member.name):
                raise ValueError(f'Unexpected archive path: {member.name}')
            total += member.size
            if total > MAX_TOTAL:
                raise ValueError('Archive exceeds the settings size limit')
            files[member.name] = archive.extractfile(member).read()
    manifest = json.loads(files.pop('manifest.json'))
    if manifest['schema'] != SCHEMA or set(manifest['roots']) != set(SECTIONS):
        raise ValueError('Unsupported settings manifest')
    if set(files) != set(manifest['files']):
        raise ValueError('Archive and manifest file lists differ')
    for name, data in files.items():
        if hashlib.sha256(data).hexdigest() != manifest['files'][name]:
            raise ValueError(f'Checksum mismatch: {name}')
    return manifest, files


def preserve_window_state(preferences, destination):
    """Archive preferences are portable; the destination owns its window state."""
    try:
        current = json.loads(read_settings(destination)) if destination.exists() else {}
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise ValueError(f'Cannot preserve window state from {destination}: invalid JSON') from error
    if not isinstance(preferences, dict) or not isinstance(current, dict):
        raise ValueError(f'Expected a JSON object for Desktop preferences: {destination}')
    for key in WINDOW_STATE:
        preferences.pop(key, None)
    # Keep the original local values (including paths), in stable file order.
    preferences.update((key, value) for key, value in current.items() if key in WINDOW_STATE)


def relocate(data, suffix, mappings, *, preferences_at=None):
    def path_value(value):
        for source, destination in mappings:
            if value == source or value.startswith(source + '/'):
                return destination + value[len(source):]
        return value

    def visit(value):
        if isinstance(value, dict):
            return {path_value(key): visit(child) for key, child in value.items()}
        if isinstance(value, list):
            return [visit(child) for child in value]
        return path_value(value) if isinstance(value, str) else value

    if suffix not in ('.json', '.toml'):
        return data
    value = structured(data, suffix)
    reject_credentials(value, 'archive')
    if suffix == '.json':
        value = visit(value)
        if preferences_at is not None:
            preserve_window_state(value, preferences_at)
        return (json.dumps(value, ensure_ascii=False, indent=2) + '\n').encode()

    def literal(match):
        token = match.group()
        if token.startswith('#'):
            return token
        # Simple printable ASCII strings need no TOML escape decoding.
        if (not token.startswith(('"""', "'''"))
                and token.isascii() and token.isprintable() and '\\' not in token):
            original = token[1:-1]
        else:
            original = tomllib.loads('value = ' + token)['value']
        moved = path_value(original)
        return json.dumps(moved, ensure_ascii=False) if moved != original else token

    text = TOML_STRING.sub(literal, data.decode('utf-8'))
    if tomllib.loads(text) != visit(value):
        raise ValueError('Cannot safely relocate this TOML representation')
    return text.encode()


def is_named_profile(path, agent):
    return (
        path.parent in (agent / 'configs', agent / 'opencode-go/configs')
        and (path.name == 'config.toml' or path.name.endswith('.config.toml'))
    )


def profile_storage(path, data_by_path, home, parents=frozenset()):
    if path in parents:
        raise ValueError(f'Circular config include: {path}')
    data = data_by_path[path] if path in data_by_path else read_settings(path)
    doc = structured(data, path.suffix)
    selected = 'session_storage' in doc.get('modules', {})
    components = doc.get('components', {})
    declared = 'session-storage' in components or any(
        'session_storage' in component.get('exports', {}) for component in components.values()
    )
    if selected:
        return selected, declared
    includes = doc.get('include', [])
    for entry in [includes] if isinstance(includes, str) else includes:
        include = home / entry[2:] if entry.startswith('~/') else path.parent / entry
        child_selected, child_declared = profile_storage(
            include.resolve(), data_by_path, home, parents | {path}
        )
        selected |= child_selected
        declared |= child_declared
    return selected, declared


def with_session_storage(path, data_by_path, home):
    data = data_by_path[path]
    selected, declared = profile_storage(path, data_by_path, home)
    if selected:
        return data
    if declared:
        raise ValueError(f'{path}: choose an existing storage export in modules.session_storage')
    text = data.decode('utf-8')
    expected = tomllib.loads(text)
    modules = expected.setdefault('modules', {})
    had_modules = bool(modules)
    modules['session_storage'] = 'jsonl'
    component = tomllib.loads(STORAGE_COMPONENT)['components']
    expected.setdefault('components', {}).update(component)
    header = re.search(r'(?m)^[ \t]*\[[ \t]*modules[ \t]*\][ \t]*(?:#[^\n]*)?$', text)
    inline = re.search(r'(?m)^[ \t]*modules[ \t]*=[ \t]*\{', text)
    if header:
        text = text[:header.end()] + '\nsession_storage = "jsonl"' + text[header.end():]
    elif inline:
        field = 'session_storage = "jsonl", ' if had_modules else 'session_storage = "jsonl"'
        text = text[:inline.end()] + field + text[inline.end():]
    else:
        text = text.rstrip() + '\n\n[modules]\nsession_storage = "jsonl"\n'
    text = text.rstrip() + '\n\n' + STORAGE_COMPONENT.lstrip()
    try:
        unchanged_settings = tomllib.loads(text) == expected
    except tomllib.TOMLDecodeError:
        unchanged_settings = False
    if not unchanged_settings:
        raise ValueError(f'{path}: add session storage to this TOML profile manually')
    return text.encode()


def restore_archive(args):
    manifest, files = read_archive(args.archive)
    home, locations = roots(args)
    mappings = sorted([(source, str(locations[kind])) for kind, source in manifest['roots'].items()]
                      + [(manifest['home'], str(home))], key=lambda pair: len(pair[0]), reverse=True)
    if any(not Path(source).is_absolute() or source == '/' for source, _ in mappings):
        raise ValueError('Invalid source roots')
    data_by_path = {}
    for name in files:
        parts = PurePosixPath(name).parts
        destination = locations[parts[1]].joinpath(*parts[2:])
        regular_path(destination)
        preferences_at = destination if name == 'files/desktop/preferences.json' else None
        data_by_path[destination] = relocate(
            files[name], destination.suffix, mappings, preferences_at=preferences_at,
        )
        # Drop the validated source payload without changing member order.
        files[name] = None
    for path in data_by_path:
        if is_named_profile(path, locations['agent']):
            data_by_path[path] = with_session_storage(path, data_by_path, home)
    apply_settings(settings_state_dir(args, home), data_by_path, args.dry_run)


def with_native_go(data):
    if b'proteus-opencode-module' not in data:
        return data
    text = data.decode('utf-8')
    expected = tomllib.loads(text)
    if expected.get('include'):
        return data
    component = expected.get('components', {}).get('opencode-model', {})
    config = expected.get('module_config', {}).get('model', {}).get('opencode', {})
    if (component.get('command') != 'proteus-opencode-module'
            or component.get('exports') != {'model': {'opencode': {}}}
            or config.get('provider') != 'opencode-go'
            or config.keys() - {'provider', 'api_key', 'database_path'}):
        return data
    # Rewrite only the stock spelling, and verify the entire TOML round trip.
    text = text.replace('[components.opencode-model]\ncommand = "proteus-opencode-module"',
                        '[components.opencode-model]\ncommand = "proteus-model-pack-module"', 1)
    text = text.replace('[module_config.model.opencode]\n',
                        '[module_config.model.opencode]\nimplementation = "opencode_go"\n', 1)
    config['implementation'] = 'opencode_go'
    component['command'] = 'proteus-model-pack-module'
    return text.encode('utf-8') if tomllib.loads(text) == expected else data


def with_idle_workers(data, stock_path):
    """Adopt shipped worker lifetimes in old flat profiles with stock launches."""
    # The migration inserts this spelling. Keep repeated launches on a byte scan.
    if re.search(rb'(?m)^\[components\.[\w-]+\]\r?\nrelease_when_idle\s*=', data):
        return data
    if not stock_path.is_file():
        return data
    text = data.decode('utf-8')
    expected = tomllib.loads(text)
    components = expected.get('components', {})
    if expected.get('include') or any('release_when_idle' in c for c in components.values()):
        return data
    stock = tomllib.loads(stock_path.read_text(encoding='utf-8'))['components']
    updated = set()
    for name, component in components.items():
        reference = stock.get(name, {})
        if reference.get('release_when_idle') is not True:
            continue
        if {k: v for k, v in reference.items() if k not in ('release_when_idle', 'exports')} != {
            k: v for k, v in component.items() if k != 'exports'
        }:
            continue
        exports = component.get('exports', {})
        if exports and all(
            entries and all(reference['exports'].get(slot, {}).get(key) == value
                            for key, value in entries.items())
            for slot, entries in exports.items()
        ):
            component['release_when_idle'] = True
            updated.add(name)
    if not updated:
        return data
    result = re.sub(r'(?m)^\[components\.([\w-]+)\](\r?\n)',
                    lambda m: m[0] + ('release_when_idle = true' + m[2] if m[1] in updated else ''), text)
    try:
        return result.encode('utf-8') if tomllib.loads(result) == expected else data
    except tomllib.TOMLDecodeError:
        return data


def upgrade_storage(args):
    home, locations = roots(args)
    agent = locations['agent']
    if args.archive:
        profiles = [args.archive.expanduser().absolute()]
        if profiles[0].suffix != '.toml':
            raise ValueError('upgrade-storage expects a TOML profile')
    else:
        profiles = sorted(
            path for directory in (agent / 'configs', agent / 'opencode-go/configs')
            for path in directory.glob('*.toml') if is_named_profile(path, agent)
        )
    if not profiles:
        raise ValueError('No Proteus TOML profiles found')
    original = {path: read_settings(path.resolve(strict=True)) for path in profiles}
    updated = {path: with_session_storage(path, original, home) for path in profiles}
    if args.operation == 'upgrade-startup':
        stock = Path(__file__).parent / 'configs/config.toml'
        updated = {path: with_idle_workers(with_native_go(data), stock) for path, data in updated.items()}
    apply_settings(settings_state_dir(args, home), updated, args.dry_run, action='update')


def _changed_settings(data_by_path):
    buffer = None
    prepared = []
    for path, data in data_by_path.items():
        if not path.exists():
            prepared.append((path, data))
            continue
        with path.open('rb') as stream:
            if buffer is None:
                buffer = memoryview(bytearray(64 * 1024))
            desired = memoryview(data)
            length = 0
            matches = True
            while count := stream.readinto(buffer):
                end = length + count
                if matches and buffer[:count] != desired[length:end]:
                    matches = False
                length = end
            # A mismatch does not stop the read before EOF: a later I/O
            # failure must still prevent the whole batch from being written.
        if not matches or length != len(data):
            prepared.append((path, data))
    return prepared


def apply_settings(state_dir, data_by_path, dry_run, action='restore'):
    prepared = _changed_settings(data_by_path)
    for path, _ in prepared:
        regular_path(path)
    if dry_run:
        print(f'Would {action} {len(prepared)} settings files')
        return
    past = 'Restored' if action == 'restore' else 'Updated'
    if not prepared:
        print(f'{past} 0 settings files')
        return

    # The startup migration usually finds no files to write.
    import shutil
    import tempfile

    backup = state_dir / 'Proteus-settings-backups' / datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    backup.mkdir(parents=True, mode=0o700)
    written = []
    try:
        for index, (destination, data) in enumerate(prepared):
            saved = backup / str(index)
            existed = destination.exists()
            if existed:
                shutil.copy2(destination, saved)
                saved.chmod(0o600)
            destination.parent.mkdir(parents=True, exist_ok=True)
            descriptor, temporary = tempfile.mkstemp(prefix='.settings-', dir=destination.parent)
            try:
                with os.fdopen(descriptor, 'wb') as output:
                    output.write(data)
                os.replace(temporary, destination)
            finally:
                Path(temporary).unlink(missing_ok=True)
            written.append((destination, saved if existed else None))
    except BaseException:
        for destination, saved in reversed(written):
            if saved:
                os.replace(saved, destination)
            else:
                destination.unlink()
        raise
    (backup / 'manifest.json').write_text(json.dumps({str(saved): str(path) for path, saved in written if saved}, indent=2) + '\n')
    print(f'{past} {len(prepared)} settings files')
    print(f'Previous files backed up at {backup}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('export', 'restore', 'upgrade-storage', 'upgrade-startup'))
    parser.add_argument('archive', type=Path, nargs='?', metavar='ARCHIVE_OR_PROFILE')
    parser.add_argument('--home', type=Path, help='home for a staged or alternate user')
    parser.add_argument('--agent-config', type=Path)
    parser.add_argument('--desktop-config', type=Path)
    parser.add_argument('--dry-run', action='store_true', help='show changes without writing')
    args = parser.parse_args()
    if args.operation in ('export', 'restore') and args.archive is None:
        parser.error('export and restore require an archive path')
    if args.dry_run and args.operation == 'export':
        parser.error('--dry-run applies to restore and upgrade-storage')
    errors = (OSError, ValueError, KeyError)
    if args.operation in ('export', 'restore'):
        from tarfile import TarError

        errors += (TarError,)
    try:
        operations = {'export': export_archive, 'restore': restore_archive, 'upgrade-storage': upgrade_storage, 'upgrade-startup': upgrade_storage}
        operations[args.operation](args)
    except errors as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
