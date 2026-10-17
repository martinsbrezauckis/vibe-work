# Synthetic stored sessions

These files follow Vibe 2.26.1's `SessionLogger._save_interaction_sync`:
`meta.json` holds one overwritten metadata/stats snapshot; `messages.jsonl`
contains non-system `LLMMessage.model_dump(exclude_none=True, mode="json")` records.
Names, IDs, paths, counters and text are synthetic. No real session is copied.

- `normal`: one completed exchange, with recorded session/last-request usage.
- `tools`: two timed tool calls, injected context and manual-shell context.
- `subagent`: a task with a linked child under `subagents/child`; two child
  model iterations within one child user turn.
- `failed`: a prior successful exchange followed by a failed tool and partial
  assistant response; the last-request snapshot still belongs to the earlier successful call.
- `without-usage`: a retained exchange without any recorded usage/config fields.

Metadata `_fixture` strings document the fixture. Messages deliberately carry
no invented model, usage, outcome or timing fields. Fault tests mutate these
fixtures in memory. The integration tests call the public streaming API.
