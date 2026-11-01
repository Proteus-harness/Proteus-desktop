#!/usr/bin/env python3
"""Synthetic Codex app-server for native UI checks; no authentication or network.

Set this executable as the Codex binary. Optional PROTEUS_CODEX_FIXTURE_STATE
persists synthetic chats to that JSON file for restart checks. Messages:
/approval, /question, /permissions, /slow, /error, /disconnect. Ordinary text
streams Markdown plus a command result. Existing chat has 600 history rows.
"""
import json
import os
import sys
import threading
import time
from pathlib import Path

if "--version" in sys.argv:
    print("codex-cli 0.162.0 (Proteus synthetic fixture)")
    raise SystemExit

write_lock = threading.Lock()
state_lock = threading.RLock()
stop = {}
pending = {}
sequence = 0
state_path = Path(os.environ["PROTEUS_CODEX_FIXTURE_STATE"]) if os.environ.get("PROTEUS_CODEX_FIXTURE_STATE") else None
cwd = os.getcwd()


def emit(value):
    with write_lock:
        print(json.dumps(value, ensure_ascii=False), flush=True)


def notify(method, params):
    emit({"method": method, "params": params})


def item(thread, turn, value, complete=True):
    notify("item/completed" if complete else "item/started", {"threadId": thread, "turnId": turn, "item": value, "startedAtMs": int(time.time() * 1000)})
    if complete:
        with state_lock:
            data[thread]["history"].append({"turnId": turn, "item": value})
            save()


def summary(thread):
    value = data[thread]
    return {"id": thread, "name": value["name"], "preview": value["name"], "cwd": value["cwd"], "createdAt": 1730000000,
            "updatedAt": value.get("updatedAt", 1730000000), "status": {"type": "active" if thread in stop else "idle"}, "model": "fixture-model", "parentThreadId": value.get("parentThreadId")}


def save():
    if state_path:
        state_path.parent.mkdir(parents=True, exist_ok=True)
        temporary = state_path.with_suffix(".new")
        temporary.write_text(json.dumps(data, ensure_ascii=False))
        temporary.replace(state_path)


def seed():
    history = []
    for index in range(300):
        turn = f"history:turn:{index}"
        history.extend([
            {"turnId": turn, "item": {"id": f"history:user:{index}", "type": "userMessage", "content": [{"type": "text", "text": f"Тестовый вопрос {index + 1}"}]}},
            {"turnId": turn, "item": {"id": f"history:assistant:{index}", "type": "agentMessage", "text": f"Синтетический ответ {index + 1}. Проверка прокрутки и восстановления истории.\n\n```rust\nlet row = {index + 1};\n```"}},
        ])
    return {"fixture:thread/main": {"name": "Codex · длинная синтетическая история", "cwd": cwd, "history": history},
            "fixture:thread/child": {"name": "Синтетический субагент", "cwd": cwd, "parentThreadId": "fixture:thread/main", "history": []}}


data = json.loads(state_path.read_text()) if state_path and state_path.exists() else seed()


def finish(thread, turn, status="completed"):
    with state_lock:
        if stop.get(thread, (None,))[0] != turn:
            return
        stop.pop(thread, None)
    notify("turn/completed", {"threadId": thread, "turn": {"id": turn, "status": status}})


def answer(thread, turn, cancel, text):
    rid = f"assistant:{turn}"
    item(thread, turn, {"id": rid, "type": "agentMessage", "text": ""}, False)
    output = ""
    for chunk in ["Это **синтетический Codex**, ", "без API и авторизации.\n\n", "```rust\nlet answer = 42;\n```\n\n", text]:
        if cancel.wait(0.2):
            finish(thread, turn, "interrupted")
            return
        output += chunk
        notify("item/agentMessage/delta", {"threadId": thread, "turnId": turn, "itemId": rid, "delta": chunk})
    item(thread, turn, {"id": rid, "type": "agentMessage", "text": output})
    finish(thread, turn)


def run(thread, turn, cancel, text):
    reasoning = f"reasoning:{turn}"
    item(thread, turn, {"id": reasoning, "type": "reasoning", "summary": []}, False)
    notify("item/reasoning/summaryTextDelta", {"threadId": thread, "turnId": turn, "itemId": reasoning, "delta": "Проверяю нативное отображение…"})
    if cancel.wait(1):
        finish(thread, turn, "interrupted")
        return
    item(thread, turn, {"id": reasoning, "type": "reasoning", "summary": ["Проверяю нативное отображение…"]})
    if "/slow" in text:
        if cancel.wait(30):
            finish(thread, turn, "interrupted")
            return
    if "/disconnect" in text:
        os._exit(0)
    if "/error" in text:
        notify("turn/completed", {"threadId": thread, "turn": {"id": turn, "status": "failed", "error": {"message": "Синтетическая ошибка хода"}}})
        with state_lock:
            stop.pop(thread, None)
        return
    kind = next((kind for kind in ["approval", "question", "permissions"] if f"/{kind}" in text), None)
    if kind:
        rid = f"fixture:request:{turn}"
        params = {"threadId": thread, "turnId": turn, "itemId": f"tool:{turn}", "startedAtMs": int(time.time() * 1000)}
        if kind == "approval":
            method = "item/commandExecution/requestApproval"
            params.update(command="printf 'synthetic command'", cwd=data[thread]["cwd"], reason="Проверка кнопок разрешения", proposedExecpolicyAmendment=["printf"])
        elif kind == "permissions":
            method = "item/permissions/requestApproval"
            params.update(permissions={"network": {"enabled": True}}, reason="Синтетический запрос сети; реальной сети нет")
        else:
            method = "item/tool/requestUserInput"
            params.update(isBlocking=True, questions=[{"id": "choice:opaque", "header": "Пример", "question": "Какой вариант показать?", "isOther": True, "options": [{"label": "Первый", "description": "Первый синтетический ответ"}, {"label": "Второй", "description": "Второй синтетический ответ"}]}])
        with state_lock:
            pending[rid] = (thread, turn, cancel)
        emit({"id": rid, "method": method, "params": params})
        return
    command = {"id": f"command:{turn}", "type": "commandExecution", "command": "printf 'fixture output'", "cwd": data[thread]["cwd"], "aggregatedOutput": "", "status": "inProgress"}
    item(thread, turn, command, False)
    notify("item/commandExecution/outputDelta", {"threadId": thread, "turnId": turn, "itemId": command["id"], "delta": "fixture output\n"})
    command.update(status="completed", aggregatedOutput="fixture output\n", exitCode=0)
    item(thread, turn, command)
    answer(thread, turn, cancel, "| Проверка | Статус |\n|---|---|\n| История | Работает |\n\nЗапрос: " + text)


for line in sys.stdin:
    try:
        message = json.loads(line)
        if "method" not in message:
            request_id = message.get("id")
            with state_lock:
                context = pending.pop(request_id, None)
            if context:
                thread, turn, cancel = context
                notify("serverRequest/resolved", {"threadId": thread, "requestId": request_id})
                threading.Thread(target=answer, args=(thread, turn, cancel, "Получен ответ: " + json.dumps(message.get("result", message.get("error")), ensure_ascii=False)), daemon=True).start()
            continue
        method = message["method"]
        params = message.get("params", {})
        if "id" not in message:
            continue
        thread = params.get("threadId")
        if method == "initialize":
            result = {"userAgent": "Proteus synthetic Codex peer"}
        elif method == "model/list":
            result = {"data": [{"id": "fixture:catalog", "model": "fixture-model", "displayName": "Synthetic Codex", "description": "Локальный тест без API", "isDefault": True, "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [{"reasoningEffort": effort} for effort in ["low", "medium", "high"]]}], "nextCursor": None}
        elif method == "thread/list":
            offset = int(params.get("cursor") or 0)
            rows = [summary(key) for key in data if not params.get("cwd") or data[key]["cwd"] == params["cwd"]]
            result = {"data": rows[offset:offset + 50], "nextCursor": str(offset + 50) if len(rows) > offset + 50 else None}
        elif method == "thread/start":
            sequence += 1
            thread = f"fixture:new/{time.time_ns()}:{sequence}"
            data[thread] = {"name": "Новый синтетический чат", "cwd": params.get("cwd", cwd), "history": []}
            save()
            result = {"thread": dict(summary(thread), turns=[]), "model": "fixture-model", "reasoningEffort": "medium"}
        elif method == "thread/resume":
            result = {"thread": dict(summary(thread), turns=[]), "model": "fixture-model", "reasoningEffort": "medium"}
        elif method == "thread/turns/list":
            result = {"data": [{"id": stop[thread][0], "status": "inProgress", "items": []}] if thread in stop else [], "nextCursor": None}
        elif method == "thread/items/list":
            offset = int(params.get("cursor") or 0)
            rows = list(reversed(data[thread]["history"]))
            result = {"data": rows[offset:offset + 50], "nextCursor": str(offset + 50) if len(rows) > offset + 50 else None}
        elif method == "turn/start":
            if thread in stop:
                raise ValueError("Synthetic thread already active")
            sequence += 1
            turn = f"fixture:turn/{sequence}"
            text = "\n".join(part.get("text", "") for part in params.get("input", []))
            cancel = threading.Event()
            stop[thread] = (turn, cancel)
            data[thread]["updatedAt"] = int(time.time())
            emit({"id": message["id"], "result": {"turn": {"id": turn, "status": "inProgress", "items": []}}})
            notify("turn/started", {"threadId": thread, "turn": {"id": turn, "status": "inProgress"}})
            item(thread, turn, {"id": f"user:{turn}", "type": "userMessage", "content": [{"type": "text", "text": text}]})
            threading.Thread(target=run, args=(thread, turn, cancel, text), daemon=True).start()
            continue
        elif method == "turn/interrupt":
            if thread in stop and stop[thread][0] == params["turnId"]:
                stop[thread][1].set()
                finish(thread, params["turnId"], "interrupted")
            result = {}
        elif method == "thread/delete":
            data.pop(thread, None)
            save()
            result = {}
        elif method == "thread/unsubscribe":
            result = {}
        else:
            emit({"id": message["id"], "error": {"code": -32601, "message": "Unknown synthetic method: " + method}})
            continue
        emit({"id": message["id"], "result": result})
    except Exception as error:
        emit({"id": message.get("id"), "error": {"code": -32000, "message": str(error)}})
