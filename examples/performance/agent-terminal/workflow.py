"""Model-free workflow: exercise the real shell tool through Core."""
import shlex
import sys
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from component_runtime import ProtocolError, run_component


def initialize(params):
    exports = []
    for export in params["exports"]:
        if (export["slot"], export["contract_version"], export["composition"]) != (
            "workflow", "v20", "select_one"
        ):
            raise ProtocolError("expected workflow/v20")
        exports.append({key: export[key] for key in (
            "slot", "module_id", "contract_version", "composition"
        )} | {"module_features": [], "config_schema": None})
    return {"protocol_version": "v3", "component_id": params["component_id"], "exports": exports}


def message(role, payload, *, tool_call_id=None):
    return {
        "id": str(uuid.uuid4()), "role": role, "phase": None,
        "parts": [{"part_id": str(uuid.uuid4()), "provenance": "tool" if role == "Tool" else "model",
                   "scope": "conversation", "payload": payload}],
        "name": None, "tool_call_id": tool_call_id, "metadata": None,
    }


def invoke(context, method, params):
    if method != "run":
        raise ProtocolError("expected run")
    call_id = "terminal-demo-" + uuid.uuid4().hex
    command = shlex.join([sys.executable, "-u", str(Path(__file__).with_name("password.py"))])
    call = {
        "id": call_id, "name": "exec_command",
        "args": {"cmd": command, "tty": True,
                 "with_escalated_permissions": "--unsandboxed" in sys.argv[1:],
                 "yield_time_ms": 1000, "max_output_tokens": 2000},
        "surface": "function", "raw_arguments": None,
    }
    result = context.host_call("host.tools.execute", {"task": params["task"], "call": call})
    if not result.get("ok"):
        raise ProtocolError("demo command failed; inspect the exec_command tool row")
    text = ("Команда создана. Кнопка терминала у exec_command открывает тот же процесс. "
            "Для повторной проверки отправьте ещё одно сообщение.")
    return {
        "status": "success",
        "result": {
            "output": {"text": text, "metadata": None},
            "new_messages": [
                message("Assistant", {"ToolCall": {"call": call}}),
                message("Tool", {"ToolResult": {"result": result}}, tool_call_id=call_id),
                message("Assistant", {"Text": {"text": text}}),
            ],
            "history_replacement": None, "compactions": [],
        },
    }


run_component(initialize, invoke)
