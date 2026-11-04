#!/usr/bin/env python3
"""Launch an already built portable in isolated state; no model or install."""
import argparse
import json
import os
from pathlib import Path
import shutil

SOURCE = Path(__file__).resolve().parent
DESKTOP = SOURCE.parents[2]


def prepare(native, output, core):
    portable = native.parent
    if not (portable / "bin/proteus").is_file():
        raise SystemExit("--native must be the ready portable launcher")
    shell = portable / "bin/proteus-shell-tool-module"
    if not shell.is_file():
        raise SystemExit("ready portable has no shell-tool executable")
    policy = portable / "bin/proteus-policy-pack-module"
    if not policy.is_file():
        raise SystemExit("ready portable has no policy-pack executable")
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    for name in ("workspace", "config", "home", "runtime", "xdg-config", "xdg-data",
                 "xdg-state", "xdg-cache", "fixtures", "empty-extensions"):
        (output / name).mkdir(mode=0o700)
    for name in ("workflow.py", "password.py"):
        shutil.copyfile(SOURCE / name, output / "fixtures" / name)
    shutil.copyfile(core / "examples/modules/component_runtime.py",
                    output / "fixtures/component_runtime.py")
    (output / "workspace/README.md").write_text(
        "Синтетическая проверка терминала.\n"
        "Отправьте любое сообщение, откройте терминал у exec_command и введите выдуманный пароль.\n"
        "Никакие пакеты не устанавливаются, модель не вызывается.\n")
    profile = output / "config/terminal-demo.toml"
    profile.write_text(
        '[profile]\nname = "terminal-demo"\n'
        '[modules]\nworkflow = "terminal_demo"\npolicy = "allow_all"\n'
        '[tools]\nenabled = ["exec_command", "write_stdin"]\n'
        '[agent_control]\nsurface = "none"\nroles = []\n'
        '[components.terminal-demo]\ncommand = "python3"\nargs = ["-B", '
        + json.dumps(str(output / "fixtures/workflow.py")) + ']\n'
        '[components.terminal-demo.exports.workflow.terminal_demo]\n'
        '[components.shell-tool]\ncommand = ' + json.dumps(str(shell)) + '\n'
        'env_allowlist = ["HOME", "PROTEUS_HOME"]\n'
        '[components.shell-tool.exports.tool.shell_tools]\n'
        '[components.demo-policy]\ncommand = ' + json.dumps(str(policy)) + '\n'
        '[components.demo-policy.exports.policy.allow_all]\n'
        '[runtime]\nworkflow_timeout_ms = 120000\n')
    return profile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, default=DESKTOP / "build/Proteus/proteus-native")
    parser.add_argument("--core-source", type=Path, default=DESKTOP.parent / "Proteus-core")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prepare-only", action="store_true")
    args = parser.parse_args()
    native = args.native.resolve(strict=True)
    output = args.output.resolve()
    profile = prepare(native, output, args.core_source.resolve(strict=True))
    keep = {"DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS",
            "XAUTHORITY", "XDG_SESSION_TYPE", "XDG_DATA_DIRS", "FONTCONFIG_PATH", "FONTCONFIG_FILE",
            "LIBGL_ALWAYS_SOFTWARE", "WGPU_BACKEND", "ZIG"}
    env = {key: value for key, value in os.environ.items() if key in keep}
    env.update({"PATH": "/usr/local/bin:/usr/bin:/bin", "LANG": "C.UTF-8",
                "HOME": str(output / "home"), "PROTEUS_HOME": str(output / "runtime"),
                "PROTEUS_CONFIG_HOME": str(output / "config"),
                "PROTEUS_EXTENSIONS_DIR": str(output / "empty-extensions"),
                "XDG_CONFIG_HOME": str(output / "xdg-config"),
                "XDG_DATA_HOME": str(output / "xdg-data"),
                "XDG_STATE_HOME": str(output / "xdg-state"),
                "XDG_CACHE_HOME": str(output / "xdg-cache")})
    command = [str(native), str(output / "workspace"), "--agent", "proteus", "--config", str(profile)]
    (output / "prepared.json").write_text(json.dumps({"command": command, "model": None,
                                                     "synthetic": True}, indent=2) + "\n")
    if args.prepare_only:
        print(output)
        return
    os.execve(native, command, env)


if __name__ == "__main__":
    main()
