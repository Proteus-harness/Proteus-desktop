"""Synthetic tool that delegates only the prepared command to real shell-tool."""
import sys
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from component_runtime import ProtocolError, run_component


def initialize(params):
    exports = []
    for export in params["exports"]:
        if (export["slot"], export["contract_version"], export["composition"]) != (
            "tool", "v6", "ordered_many"
        ):
            raise ProtocolError("expected tool/v6")
        exports.append({key: export[key] for key in (
            "slot", "module_id", "contract_version", "composition"
        )} | {"module_features": [], "config_schema": None})
    return {"protocol_version": "v3", "component_id": params["component_id"], "exports": exports}


def invoke(context, method, params):
    if method == "list":
        return {"result": [{"spec": {
            "name": "terminal_proxy", "description": "Synthetic nested terminal probe",
            "input_schema": {"type": "object", "properties": {}},
            "surface": {"kind": "function", "strict": False, "output_schema": None},
            "safety": "RunsCommands", "supports_parallel_tool_calls": False,
            "timeout_ms": None, "metadata": {},
        }, "model_visible": True, "user_command": None}]}
    if method != "invoke":
        raise ProtocolError("expected tool invoke")
    outer = params["call"]
    child = dict(outer, id="nested-terminal-" + uuid.uuid4().hex, name="exec_command")
    result = context.host_call("host.tools.execute", {"call": child})
    # The workflow receives only its own result. Child activity/terminal must
    # come from Core's recorded UI projection, never fabricated history parts.
    return {"result": dict(result, call_id=outer["id"], metadata={
        "demo_nested_call_id": child["id"]
    })}


run_component(initialize, invoke)
