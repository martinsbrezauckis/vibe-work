"""Intermediate representation between Vibe's Pydantic models and the two code emitters.

Role: a small, language-neutral description of every protocol type. `introspect.py` builds it from
the installed Vibe package; `rust_emit.py` and `ts_emit.py` only read it. Keeping the emitters
ignorant of Pydantic is what lets both targets share one set of semantics for optional-versus-null,
tagged unions and unknown-variant handling.
Called by: introspect.py (producer), areas.py, rust_emit.py, ts_emit.py, registry_emit.py.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Prim:
    """A JSON primitive. `kind` is one of: string, int, float, bool, json (any JSON value)."""

    kind: str


@dataclass(frozen=True)
class Lit:
    """A single-value literal (`Literal["x"]`), e.g. a constant `jsonrpc: "2.0"` field."""

    value: str | int | bool


@dataclass(frozen=True)
class ListOf:
    """A JSON array whose items are all `item`."""

    item: TypeRef


@dataclass(frozen=True)
class MapOf:
    """A JSON object with string keys and uniform `value` type."""

    value: TypeRef


@dataclass(frozen=True)
class Nullable:
    """`inner | null` nested inside a container (field-level nullability is on `Field`)."""

    inner: TypeRef


@dataclass(frozen=True)
class Ref:
    """A reference to a named definition (model, enum, tagged union or untagged union)."""

    name: str


TypeRef = Prim | Lit | ListOf | MapOf | Nullable | Ref


@dataclass
class Field:
    """One property of a model.

    `required` and `nullable` follow Pydantic exactly and are independent: a field may be absent
    (not required), may be null (nullable), neither, or both. `discriminator` marks the constant
    tag of a tagged-union member; such fields are carried by the union, not by the struct.
    """

    py_name: str
    wire: str
    type: TypeRef
    required: bool
    nullable: bool
    doc: str = ""
    discriminator: bool = False
    boxed: bool = False


@dataclass
class ModelDef:
    """A Pydantic model rendered as a struct / interface."""

    name: str
    fields: list[Field]
    doc: str
    source: str  # "vibe/app_server/models.py:123", cited in the generated doc comment


@dataclass
class EnumDef:
    """A closed set of string values (StrEnum or multi-value Literal)."""

    name: str
    values: list[str]
    doc: str
    source: str


@dataclass
class UnionDef:
    """A discriminated union: `tag` is the wire property, `members` maps tag value to model name."""

    name: str
    tag: str
    members: list[tuple[str, str]]
    doc: str
    source: str
    named: bool = False  # True when Vibe itself names this union (a module-level alias)


@dataclass
class UntaggedDef:
    """A union without a discriminator; members are tried in order when decoding."""

    name: str
    members: list[TypeRef]
    doc: str
    source: str
    named: bool = False


Def = ModelDef | EnumDef | UnionDef | UntaggedDef


@dataclass
class Schema:
    """Everything the emitters need: definitions by name, plus the method registry."""

    defs: dict[str, Def] = field(default_factory=dict)
    # method registry: wire method name -> (params type name | None, result type name | None)
    requests: dict[str, tuple[str | None, str | None]] = field(default_factory=dict)
    notifications: dict[str, str | None] = field(default_factory=dict)
    server_requests: dict[str, tuple[str | None, str | None]] = field(default_factory=dict)
    vibe_version: str = ""
