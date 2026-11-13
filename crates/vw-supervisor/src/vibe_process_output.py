"""Plain-text native process replies for the agent, installed by the pinned launcher.

Wraps the runtime's process.output result only. The process store and its byte cursors,
including the raw terminal stream read by the UI, remain owned by Vibe.
"""


def process_output_text(text):
    """Remove terminal controls while retaining Unicode, newlines, carriage returns and tabs."""
    output = []
    index = 0
    while index < len(text):
        char = text[index]
        index += 1
        if char == "\x1b":
            if index == len(text):
                break
            char = text[index]
            index += 1
            if char == "[":
                char = "\x9b"
            elif char in "]PX^_":
                while index < len(text):
                    if text[index] == "\x07":
                        index += 1
                        break
                    if text[index:index + 2] == "\x1b\\":
                        index += 2
                        break
                    index += 1
                continue
            elif char in "()*+":
                index += 1
                continue
            else:
                continue
        if char == "\x9b":
            while index < len(text):
                final = text[index]
                index += 1
                if "@" <= final <= "~":
                    break
        elif char in "\n\r\t" or not (ord(char) < 32 or 127 <= ord(char) <= 159):
            output.append(char)
    return "".join(output)


def _output_content(event):
    content = getattr(getattr(event, "result", None), "structured_content", None)
    if isinstance(content, dict) and isinstance(content.get("output"), str):
        return content
    return None


# On Windows the pseudo console holds a program's first output for about 3 s after it
# starts (its startup terminal query is not answered by the runtime's PTY host); with the
# shell's own startup the text reaches the process log about 3.6 s after the recorded
# start. A waiting read that has seen only terminal controls so far therefore waits until
# this many seconds after the process started, even when the caller asked for less.
WINDOWS_FIRST_OUTPUT_FLOOR_S = 5.0


async def _seconds_since_start(runtime, request):
    """Seconds since the runtime recorded the process as started, or None if unknown."""
    from datetime import datetime, timezone

    process = await runtime._require_process(request.process_id)
    if not process.started_at:
        return None
    started = datetime.fromisoformat(process.started_at.replace("Z", "+00:00"))
    return (datetime.now(timezone.utc) - started).total_seconds()


def process_output_reader(original, clock=None, first_output_floor=0.0, started_ago=None):
    """Wrap a runtime process.output reader so the agent receives plain text.

    The runtime answers a waiting read as soon as any byte follows the cursor. Terminal
    controls can arrive on their own before a program's text (the Windows pseudo console
    writes a mode preamble first; a program can hide the cursor before it prints), so such
    a page holds only controls and is empty once filtered. A filtered page that is empty is
    therefore followed by further reads from its end, inside the caller's wait and byte
    budget, until visible text arrives, the process stops, or the wait ends. Text that
    arrives after the wait is left for the next read. The bytes and cursors of every read
    are the runtime's own; the agent's nextCursor stays exact.

    With a first_output_floor (seconds), such a read waits at least until that long after
    the process started, so a console that holds the first output past a short wait still
    yields the text in the same read. The floor bounds the extra wait: it never extends a
    read past the floor, and a read without a wait (waitMs 0) stays immediate.
    """
    from dataclasses import replace
    from functools import wraps
    import time

    clock = clock or time.monotonic
    started_ago = started_ago or _seconds_since_start

    @wraps(original)
    async def read_output(self, action, request, manager):
        deadline = clock() + request.wait_ms / 1000
        event = await original(self, action, request, manager)
        content = _output_content(event)
        if content is None:
            return event
        first = content
        floor_checked = not first_output_floor
        while (
            content["output"]
            and process_output_text(content["output"]) == ""
            and not request.from_end
            and request.wait_ms > 0
            and content.get("status") == "running"
            and isinstance(content.get("nextCursor"), int)
        ):
            if not floor_checked:
                floor_checked = True
                try:
                    ago = await started_ago(self, request)
                except Exception:
                    ago = None
                if ago is not None:
                    floor_left = min(first_output_floor, max(0.0, first_output_floor - ago))
                    deadline = max(deadline, clock() + floor_left)
            remaining_ms = int((deadline - clock()) * 1000)
            budget = request.max_bytes - len(content["output"].encode("utf-8"))
            if remaining_ms <= 0 or budget <= 0:
                break
            follow = replace(
                request,
                cursor=content["nextCursor"],
                wait_ms=remaining_ms,
                max_bytes=budget,
            )
            try:
                more_event = await original(self, action, follow, manager)
            except Exception:
                # The first read already succeeded; a failed extra read keeps that answer.
                break
            more = _output_content(more_event)
            if more is None:
                break
            event = more_event
            content = {
                **more,
                "output": content["output"] + more["output"],
                "truncatedBefore": first.get("truncatedBefore", False),
            }
        cleaned = process_output_text(content["output"])
        if content is first and cleaned == content["output"]:
            return event
        result = event.result.model_copy(update={
            "structured_content": {**content, "output": cleaned},
        })
        return event.model_copy(update={"result": result})

    return read_output


def install_process_output():
    import sys
    from mistralai_vibe_local_harness.vibe._runtime import DurableSessionRuntime

    DurableSessionRuntime._read_process_output = process_output_reader(
        DurableSessionRuntime._read_process_output,
        first_output_floor=WINDOWS_FIRST_OUTPUT_FLOOR_S if sys.platform == "win32" else 0.0,
    )
