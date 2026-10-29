"""Fixed wire scenario and independent, literal text expectations."""
from fixture_server import SESSION_ID

CHILD_ID = "00000000-0000-0000-0000-000000000030"
CHILD_TURN_ID = "00000000-0000-0000-0000-000000000031"
SECOND_ID = "00000000-0000-0000-0000-000000000101"
STAGES = ("burst", "barrier", "continuation", "hold")
COPY_ROWS = {"barrier": ("primary",), "continuation": ("primary",),
             "hold": ("primary", "secondary")}
BOOT = "NATIVE STREAM READY"
CONTINUATION = " Продолжение после инструмента."
SECOND = "DISTINCT MESSAGE: другой ответ 🌍"


def chunk(index):
    return f"Ж{index:02d} "


# These expected strings are constructed independently of the delta reducer.
PRIMARY = ("".join(chunk(i) for i in range(8)) + "ИСПРАВЛЕНО 🦀 "
           + "".join(chunk(i) for i in range(33, 64)))
EXPECTED = {
    "burst": {"primary": PRIMARY},
    "barrier": {"primary": PRIMARY},
    "continuation": {"primary": PRIMARY + CONTINUATION, "secondary": SECOND},
    "hold": {"primary": PRIMARY + CONTINUATION, "secondary": SECOND},
}


def emit_stage(fixture, stage):
    if stage == "burst":
        fixture.run_id = "run-synthetic-native-stream"
        fixture.execution("running")
        # Root TurnStarted was already included in the subscription baseline:
        # root_thread_id from the snapshot must establish routing on its own.
        offset = 0
        for index in range(64):
            if index == 32:
                # Same message ID deliberately: routing must reject child text
                # before it can truncate the root row or displace its turn.
                fixture.runtime("TurnStarted", {"session_id": SESSION_ID,
                                "thread_id": CHILD_ID, "turn_id": CHILD_TURN_ID},
                                CHILD_ID, CHILD_TURN_ID)
                fixture.delta(0, "CHILD MUST NEVER APPEAR", thread_id=CHILD_ID,
                              turn_id=CHILD_TURN_ID)
                offset = len("".join(chunk(i) for i in range(8)).encode())
                text = "ИСПРАВЛЕНО 🦀 "
            else:
                text = chunk(index)
            fixture.delta(offset, text)
            offset += len(text.encode())
    elif stage == "barrier":
        fixture.tool("barrierprobe")
    elif stage == "continuation":
        fixture.delta(len(PRIMARY.encode()), CONTINUATION)
        fixture.delta(0, "DISTINCT MESSAGE: ", message_id=SECOND_ID)
        fixture.delta(len("DISTINCT MESSAGE: ".encode()), "другой ответ 🌍",
                      message_id=SECOND_ID)
    elif stage == "hold":
        # Native hides message Copy while streaming. This barrier exposes it
        # without supplying replacement text or healing a dropped delta.
        fixture.tool("freezeprobe")
    else:
        raise ValueError("Unknown stage: " + stage)
