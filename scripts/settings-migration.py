#!/usr/bin/env python3
"""Export Proteus settings and restore them on another machine (Python 3.11+)."""
import argparse
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import sys
import tarfile
import tempfile
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


def export_archive(args):
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
            for path in candidates:
                if path.is_symlink():
                    raise ValueError(f'Symlink is not supported: {path}')
                relative = path.relative_to(locations[kind])
                if path.is_dir() or EXCLUDED.intersection(relative.parts):
                    continue
                name = f'files/{kind}/{path.relative_to(locations[kind]).as_posix()}'
                data = read_settings(path)
                if path.suffix in ('.json', '.toml'):
                    reject_credentials(structured(data, path.suffix), path)
                if kind == 'desktop' and item == 'preferences.json':
                    preferences = json.loads(data)
                    for key in WINDOW_STATE:
                        preferences.pop(key, None)
                    data = (json.dumps(preferences, ensure_ascii=False, indent=2) + '\n').encode()
                files[name] = data
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
    for name, data in files.items():
        parts = PurePosixPath(name).parts
        destination = locations[parts[1]].joinpath(*parts[2:])
        regular_path(destination)
        preferences_at = destination if name == 'files/desktop/preferences.json' else None
        data_by_path[destination] = relocate(
            data, destination.suffix, mappings, preferences_at=preferences_at,
        )
    for path in data_by_path:
        if is_named_profile(path, locations['agent']):
            data_by_path[path] = with_session_storage(path, data_by_path, home)
    apply_settings(settings_state_dir(args, home), data_by_path, args.dry_run)


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
    apply_settings(settings_state_dir(args, home), updated, args.dry_run, action='update')


def apply_settings(state_dir, data_by_path, dry_run, action='restore'):
    prepared = [
        (path, data) for path, data in data_by_path.items()
        if not path.exists() or path.read_bytes() != data
    ]
    for path, _ in prepared:
        regular_path(path)
    if dry_run:
        print(f'Would {action} {len(prepared)} settings files')
        return
    backup = state_dir / 'Proteus-settings-backups' / datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    if prepared:
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
    if prepared:
        (backup / 'manifest.json').write_text(json.dumps({str(saved): str(path) for path, saved in written if saved}, indent=2) + '\n')
    past = 'Restored' if action == 'restore' else 'Updated'
    print(f'{past} {len(prepared)} settings files')
    if prepared:
        print(f'Previous files backed up at {backup}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('export', 'restore', 'upgrade-storage'))
    parser.add_argument('archive', type=Path, nargs='?', metavar='ARCHIVE_OR_PROFILE')
    parser.add_argument('--home', type=Path, help='home for a staged or alternate user')
    parser.add_argument('--agent-config', type=Path)
    parser.add_argument('--desktop-config', type=Path)
    parser.add_argument('--dry-run', action='store_true', help='show changes without writing')
    args = parser.parse_args()
    if args.operation != 'upgrade-storage' and args.archive is None:
        parser.error('export and restore require an archive path')
    if args.dry_run and args.operation == 'export':
        parser.error('--dry-run applies to restore and upgrade-storage')
    try:
        operations = {'export': export_archive, 'restore': restore_archive, 'upgrade-storage': upgrade_storage}
        operations[args.operation](args)
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
