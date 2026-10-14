"""Identifier helpers shared by the Rust and TypeScript emitters.

Role: turn wire strings (enum values, method names, Python field names) into legal, readable
identifiers in the target languages. Depends on: the standard library only.
Called by: rust_emit.py, ts_emit.py, registry_emit.py. All functions are pure and deterministic,
which the "regeneration produces no diff" check relies on.
"""

from __future__ import annotations

import re

# Rust strict and reserved keywords that can appear as Python field names or enum values.
RUST_KEYWORDS = frozenset(
    "as async await break const continue crate dyn else enum extern false fn for if impl in let "
    "loop match mod move mut pub ref return self Self static struct super trait true type unsafe "
    "use where while abstract become box do final macro override priv typeof unsized virtual yield "
    "try gen".split()
)
# Keywords that cannot be written as raw identifiers (r#self is illegal).
RUST_NON_RAW = frozenset({"self", "Self", "super", "crate"})


def split_words(text: str) -> list[str]:
    """Split on non-alphanumerics and lower-to-upper camel boundaries ("needs_auth" -> 2 words)."""
    spaced = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", text)
    return [w for w in re.split(r"[^A-Za-z0-9]+", spaced) if w]


def pascal(text: str) -> str:
    """PascalCase for type and variant names; a leading digit gets a `V` prefix."""
    out = "".join(w[:1].upper() + w[1:] for w in split_words(text))
    if not out:
        return "Empty"
    return "V" + out if out[0].isdigit() else out


def snake(text: str) -> str:
    """snake_case for Rust fields and TS-neutral keys."""
    return "_".join(w.lower() for w in split_words(text)) or "empty"


def rust_field_ident(python_name: str) -> str:
    """Rust field identifier for a Python field name, raw-escaped when it is a keyword."""
    name = snake(python_name) if not python_name.isidentifier() else python_name
    if name in RUST_NON_RAW:
        return name + "_"
    return "r#" + name if name in RUST_KEYWORDS else name


def rust_variant_ident(wire: str) -> str:
    """Rust enum variant name for a wire value."""
    name = pascal(wire)
    return name + "_" if name in RUST_NON_RAW else name


def method_type_name(method: str) -> str:
    """PascalCase marker name for a JSON-RPC method ("session/turn/queue/read" -> ...)."""
    return pascal(method)


def area_of(method: str) -> str:
    """Protocol area (file stem) a method belongs to: its first path segment, normalised."""
    return snake(method.split("/")[0])
