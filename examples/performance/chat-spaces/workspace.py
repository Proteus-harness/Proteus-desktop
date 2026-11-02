"""Read-only synthetic workspace DTOs matching app_server/http/workspace.rs."""
from pathlib import Path


def addressed_path(state, relative):
    root = Path(state.workspace).resolve()
    path = (root / relative).resolve(strict=True)
    if not path.is_relative_to(root):
        raise ValueError("Path must belong to the synthetic session workspace")
    return path


def listing(state, relative):
    root = Path(state.workspace)
    path = addressed_path(state, relative)
    entries = [{"name": entry.name, "path": str(entry.relative_to(root)),
                "kind": "directory" if entry.is_dir() else "file"}
               for entry in path.iterdir()]
    entries.sort(key=lambda entry: (entry["kind"] != "directory", entry["name"].lower()))
    return {"path": relative, "entries": entries, "truncated": False}


def read(state, relative):
    path = addressed_path(state, relative)
    if not path.is_file():
        raise ValueError("Expected a regular synthetic file")
    size = path.stat().st_size
    result = {"path": relative, "size": size, "kind": "too_large", "text": None}
    if size <= 512 * 1024:
        data = path.read_bytes()
        try:
            text = data.decode("utf-8") if b"\0" not in data else None
        except UnicodeDecodeError:
            text = None
        result.update(kind="text" if text is not None else "binary", text=text)
    return result


def changes():
    return {"repository": False, "branch": None, "head": None, "upstream": None,
            "ahead": 0, "behind": 0, "added": 0, "deleted": 0,
            "entries": [], "truncated": False}
