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


def install_process_output():
    from functools import wraps
    from mistralai_vibe_local_harness.vibe._runtime import DurableSessionRuntime

    original = DurableSessionRuntime._read_process_output

    @wraps(original)
    async def read_output(self, action, request, manager):
        event = await original(self, action, request, manager)
        content = event.result.structured_content
        if isinstance(content, dict) and isinstance(content.get("output"), str):
            cleaned = process_output_text(content["output"])
            if cleaned != content["output"]:
                result = event.result.model_copy(update={
                    "structured_content": {**content, "output": cleaned},
                })
                return event.model_copy(update={"result": result})
        return event

    DurableSessionRuntime._read_process_output = read_output
