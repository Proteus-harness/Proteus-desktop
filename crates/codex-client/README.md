# Codex transport

Native, headless JSONL client for an owned `codex app-server --listen stdio://`.
It uses Codex's own configuration/authentication. No Node runtime, Core import,
account-file parser, history cache, or automatic request replay is involved.

`Client` is cloneable; all clones share one child. `Client::spawn` completes the
initialize/initialized handshake. Run it and blocking `request` calls off the UI
thread. Take the event receiver exactly once, then route requests/notifications
by their actual thread and turn IDs in the application's service. The reader
does not wait for approval responses, so an interrupt can run while an approval
is displayed. Return unsupported server requests with an explicit `RpcError`.

`Launch::codex` supplies the standard arguments; `Launch.args` is otherwise the
complete argument vector after `program`, allowing an explicit launcher wrapper.

Limits are per transport, configurable on `Launch`: 8 MiB per JSONL frame,
16 MiB queued bytes independently in each direction, 128 queued frames and 32
pending calls by default. Event payloads move out of the parser rather than
being cloned. Received values belong to the caller after delivery; consumers
must bound their own history/UI retention. Outbound saturation rejects the new
operation. Inbound saturation, invalid frames, EOF, and I/O failure disconnect
and fail all pending calls; lifecycle events are never silently skipped to keep
the process alive. A closed event receiver is authoritative even if a full
queue could not fit the final `Disconnected` marker. Stderr is discarded rather
than accumulated or logged alongside protocol/account data.

A timed-out request has an unknown delivery/outcome. Its late reply is ignored;
the command is never sent again. Stop a turn using the normal `turn/interrupt`
request. `shutdown` or dropping the last client kills and reaps only the owned
app-server process, then joins its I/O threads. It does not search for or kill
other Codex processes.

`cargo test -p proteus-codex-client` exercises an account-free Python peer in
`tests/peer.py`, including concurrent replies, approvals, timeout, bounds and
process cleanup. The peer never reads real Codex configuration or sessions.
