"""Group definitions into per-area output files that stay well under the repository size limit.

Role: decides which generated file each definition lands in. A definition belongs to the protocol
area (first method segment) of every method that reaches it; reached from several areas it goes to
`common`. Oversized areas are split into numbered parts so no file approaches the 800-line ceiling.
Depends on: ir.py, naming.py. Called by: generate.py. Pure and deterministic.
"""

from __future__ import annotations

import re
from collections import defaultdict

from ir import Def, EnumDef, ListOf, MapOf, ModelDef, Nullable, Ref, Schema, TypeRef, UnionDef, UntaggedDef
from naming import area_of, snake

# Wire-envelope models are not tied to a method.
ENVELOPE_AREA = "envelope"
COMMON_AREA = "common"


def refs_of(d: Def) -> list[str]:
    """Names of all definitions `d` refers to (any depth)."""

    def walk(t: TypeRef) -> list[str]:
        if isinstance(t, Ref):
            return [t.name]
        if isinstance(t, Nullable):
            return walk(t.inner)
        if isinstance(t, ListOf):
            return walk(t.item)
        if isinstance(t, MapOf):
            return walk(t.value)
        return []

    if isinstance(d, ModelDef):
        return [n for f in d.fields for n in walk(f.type)]
    if isinstance(d, UnionDef):
        return sorted({m for _, m in d.members})
    if isinstance(d, UntaggedDef):
        return [n for t in d.members for n in walk(t)]
    return []


def estimate_lines(d: Def) -> int:
    """Rough emitted size of a definition, used only to balance file splits."""
    if isinstance(d, ModelDef):
        return 8 + 5 * len(d.fields)
    if isinstance(d, (EnumDef, UnionDef)):
        return 8 + 2 * len(d.values if isinstance(d, EnumDef) else d.members)
    return 12


def assign_areas(schema: Schema) -> dict[str, str]:
    """Map every definition name to its area name (before splitting)."""
    roots: dict[str, set[str]] = defaultdict(set)

    def add(name: str | None, area: str) -> None:
        if name:
            roots[name].add(area)

    for method, (params, result) in schema.requests.items():
        add(params, area_of(method))
        add(result, area_of(method))
    for method, params in schema.notifications.items():
        add(params, area_of(method))
    for method, (params, result) in schema.server_requests.items():
        add(params, area_of(method))
        add(result, area_of(method))

    reached: dict[str, set[str]] = defaultdict(set)
    for root, areas in roots.items():
        stack = [root]
        seen: set[str] = set()
        while stack:
            name = stack.pop()
            if name in seen:
                continue
            seen.add(name)
            reached[name] |= areas
            stack.extend(refs_of(schema.defs[name]))
    placement: dict[str, str] = {}
    for name in schema.defs:
        areas = reached.get(name, set())
        if len(areas) == 1:
            placement[name] = next(iter(areas))
        elif not areas:
            placement[name] = ENVELOPE_AREA
        else:
            placement[name] = f"{COMMON_AREA}_{shared_group(schema.defs[name])}"
    return placement


def shared_group(d: Def) -> str:
    """Topic of a type shared by several areas, taken from the Vibe module that defines it."""
    match = re.search(r"vibe/(?:[a-z_]+/)*_?([a-z_]+)\.py", d.source)
    return snake(match.group(1)) if match else "inline"


def split_files(schema: Schema, max_lines: int = 450) -> dict[str, list[str]]:
    """File stem -> sorted definition names. Areas over `max_lines` become `<area>_1`, `<area>_2`..."""
    buckets: dict[str, list[str]] = defaultdict(list)
    for name, area in assign_areas(schema).items():
        buckets[area].append(name)
    files: dict[str, list[str]] = {}
    for area, names in sorted(buckets.items()):
        names.sort()
        parts: list[list[str]] = [[]]
        used = 0
        for name in names:
            size = estimate_lines(schema.defs[name])
            if used + size > max_lines and parts[-1]:
                parts.append([])
                used = 0
            parts[-1].append(name)
            used += size
        if len(parts) == 1:
            files[snake(area)] = parts[0]
        else:
            for i, part in enumerate(parts, 1):
                files[f"{snake(area)}_{i}"] = part
    return files
