"""The Vibe releases the launcher guards were checked against: one list for every guard that patches Vibe.

Prepended to the launcher adapter (launch.rs `CHILD_STOP_ADAPTER`), the plugin inventory (vw-core-api
market_discovery.rs) and the Windows agent shell PATH reader (vw-core-api secret_env.rs). A Vibe outside the list
stops with a clear message instead of running without a guard. A Vibe update adds its version here only after every
guard was checked against it; a test compares the list with the pinned version.
"""
GUARDED_VIBE_VERSIONS = frozenset({"3.1.4"})
# The first words of the refusal; the supervisor shows this line as the reason Vibe did not start, and the setup
# screen recognises it as "this Vibe version is not supported yet" (vw-vibe-install `UNSUPPORTED_VIBE`).
GUARD_REFUSAL = "Vibe Work does not support Vibe "


def require_guarded_vibe(guard):
    """Stop unless the installed Vibe is a release `guard` was checked against (SystemExit: exit 1 with the message).

    The user reads one plain line (a hand-updated Vibe waits for a Vibe Work update); which guard stopped the start
    goes to stderr above it, for the log only.
    """
    import importlib.metadata
    import sys

    version = importlib.metadata.version("mistral-vibe")
    if version not in GUARDED_VIBE_VERSIONS:
        supported = ", ".join(sorted(GUARDED_VIBE_VERSIONS))
        print(f"vibe-work: not checked with Vibe {version}: {guard}", file=sys.stderr, flush=True)
        raise SystemExit(f"{GUARD_REFUSAL}{version} yet: it works with Vibe {supported}. "
                         "Wait for a Vibe Work update, or use the Vibe version Vibe Work installs.")
