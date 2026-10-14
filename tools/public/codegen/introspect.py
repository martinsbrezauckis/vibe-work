"""Build the language-neutral schema (ir.py) from Vibe's installed Pydantic models.

Role: the only module that knows Pydantic. It starts from the model names in method_map.json,
walks every reachable annotation and records models, string enums, discriminated unions and
untagged unions. Optional-versus-null is read straight from Pydantic (`is_required()` and the
presence of `None` in the annotation) and never approximated.
Depends on: the Vibe package (importable in the interpreter that runs generate.py), ir.py, naming.py.
Called by: generate.py.
Invariants: output is deterministic (definitions are sorted by name by the caller); names are
unique across Vibe modules or generation fails loudly; every model the walk meets is registered.
"""

from __future__ import annotations

import enum
import importlib
import inspect
import types
import typing
from datetime import datetime
from pathlib import Path
from typing import Any

from pydantic import BaseModel, JsonValue
from pydantic.alias_generators import to_camel
from pydantic.fields import FieldInfo

from ir import (
    Def,
    EnumDef,
    Field,
    ListOf,
    Lit,
    MapOf,
    ModelDef,
    Nullable,
    Prim,
    Ref,
    Schema,
    TypeRef,
    UnionDef,
    UntaggedDef,
)
from naming import pascal

# Modules searched (in this order) for module-level union aliases, to give unions their Vibe name.
ALIAS_MODULES = (
    "vibe.app_server.models",
    "vibe.app_server.protocol",
    "vibe.app_server._effect_models",
    "vibe.app_server.review",
    "vibe.app_server.config",
    "vibe.app_server._connection_protocol",
)
LOOKUP_MODULES = ("vibe.app_server.protocol", "vibe.app_server.models") + ALIAS_MODULES[2:]

# Models outside the method map that clients still see on the wire (JSON-RPC envelopes and errors).
ENVELOPE_ROOTS = (
    "Notification",
    "ServerRequest",
    "JsonRpcSuccessResponse",
    "JsonRpcErrorResponse",
    "ProtocolError",
    "InvalidParamsData",
)


# Vibe model names that would shadow DOM globals (TS) when imported unqualified.
MODEL_RENAMES = {"Notification": "JsonRpcNotification", "ServerRequest": "JsonRpcServerRequest"}

# Friendlier names for synthetic unions that several envelope models share.
NAME_OVERRIDES = {
    "JsonRpcServerRequestId": "RequestId",
    "JsonRpcSuccessResponseId": "RequestId",
    "JsonRpcErrorResponseId": "RequestId",
}


class GenerationError(Exception):
    """Raised when Vibe's models use a construct the generator does not (yet) understand."""


def _doc_of(obj: Any) -> str:
    """The object's own docstring (never an inherited one), dedented; empty when absent."""
    return inspect.cleandoc(obj.__dict__.get("__doc__") or "") if hasattr(obj, "__dict__") else ""


def _package_root() -> Path:
    """The folder that contains the installed `vibe` package (its parent directory)."""
    package = importlib.import_module("vibe")
    return Path(inspect.getsourcefile(package) or package.__file__ or ".").resolve().parent.parent


def _source_of(obj: Any) -> str:
    """`vibe/...py:LINE` for classes; `vibe/...py` for aliases whose line is unknown."""
    module = importlib.import_module(getattr(obj, "__module__", "vibe"))
    file = Path(inspect.getsourcefile(module) or module.__name__).resolve()
    # Relative to the folder that holds the installed `vibe` package (site-packages), never by searching the path
    # for a part named "vibe": a venv folder called `vibe` (GitHub CI uses `$RUNNER_TEMP/vibe`) matched first and
    # put `vibe/lib/python3.12/site-packages/` into every citation.
    root = _package_root()
    rel = file.relative_to(root).as_posix() if file.is_relative_to(root) else file.name
    if inspect.isclass(obj):
        try:
            return f"{rel}:{inspect.getsourcelines(obj)[1]}"
        except OSError:
            pass
    return rel


class Introspector:
    """Walks annotations and accumulates definitions. One instance per generation run."""

    def __init__(self) -> None:
        self.defs: dict[str, Def] = {}
        self._models: dict[type, str] = {}
        self.classes = self._models  # model class -> definition name (read by generate.py)
        self._aliases: list[tuple[Any, str]] = []
        for mod_name in ALIAS_MODULES:
            module = importlib.import_module(mod_name)
            for name, value in vars(module).items():
                if isinstance(value, typing.TypeAliasType) and value is not JsonValue:
                    self._aliases.append((value, value.__name__))
                elif typing.get_origin(value) is typing.Annotated and _is_union(typing.get_args(value)[0]):
                    self._aliases.append((value, name))

    # ---- public entry points -------------------------------------------------------------

    def resolve_name(self, name: str) -> Any:
        """Find a Vibe model, enum or alias by name in the protocol modules."""
        for mod_name in LOOKUP_MODULES:
            value = getattr(importlib.import_module(mod_name), name, None)
            if value is not None:
                return value
        raise GenerationError(f"mapped model {name!r} does not exist in the installed Vibe")

    def add_root(self, name: str) -> str:
        """Register a mapped model by name and return the name of its definition."""
        ref, nullable = self.convert(self.resolve_name(name), MODEL_RENAMES.get(name, name))
        if nullable or not isinstance(ref, Ref):
            raise GenerationError(f"root {name!r} must be a plain named type, got {ref!r}")
        return ref.name

    def finish(self) -> dict[str, Def]:
        """Mark union tags on members, then compute which fields need boxing (recursion)."""
        self._dedupe_unions()
        self._order_untagged()
        self._mark_discriminators()
        self._mark_boxes()
        return self.defs

    def _order_untagged(self) -> None:
        """Try the member with the most required fields first.

        serde decodes untagged enums by trying members in order and ignoring unknown fields, so a
        member whose required fields are a subset of another's must come after it or it would
        swallow the larger shape. Fails when two models could shadow each other.
        """

        def required(ref: TypeRef) -> frozenset[str]:
            d = self.defs.get(ref.name) if isinstance(ref, Ref) else None
            return frozenset(f.wire for f in d.fields if f.required) if isinstance(d, ModelDef) else frozenset()

        for d in self.defs.values():
            if not isinstance(d, UntaggedDef):
                continue
            d.members.sort(key=lambda m: -len(required(m)))
            for i, first in enumerate(d.members):
                for later in d.members[i + 1 :]:
                    if isinstance(first, Ref) and isinstance(later, Ref) and required(first) and required(first) == required(later):
                        raise GenerationError(f"{d.name}: members {first.name} and {later.name} are indistinguishable")

    def _dedupe_unions(self) -> None:
        """Merge unions of identical shape that were reached under different synthetic names.

        The canonical name prefers one Vibe gave the union itself, then the shortest, then
        alphabetical, so the result does not depend on traversal order.
        """
        groups: dict[tuple[Any, ...], list[Def]] = {}
        for d in self.defs.values():
            if isinstance(d, (UnionDef, UntaggedDef)):
                groups.setdefault(_shape(d), []).append(d)
        rename: dict[str, str] = {}
        for group in groups.values():
            group.sort(key=lambda d: (not d.named, len(d.name), d.name))
            canonical = group[0].name
            for d in group:
                rename[d.name] = canonical
        for old, new in rename.items():
            if old != new:
                moved = self.defs.pop(old)
                moved.name = new
                self.defs[new] = moved
        for d in self.defs.values():
            if isinstance(d, ModelDef):
                for f in d.fields:
                    f.type = _rename_type(f.type, rename)
            elif isinstance(d, UntaggedDef):
                d.members = [_rename_type(t, rename) for t in d.members]

    # ---- conversion ----------------------------------------------------------------------

    def _lookup_alias(self, t: Any) -> str | None:
        for value, name in self._aliases:
            if value is t or (not isinstance(t, type) and value == t):
                return name
        return None

    def convert(
        self, t: Any, ctx: str, alias_hint: str | None = None, field_disc: str | None = None
    ) -> tuple[TypeRef, bool]:
        """Convert one annotation to (type, nullable). `ctx` names synthetic definitions.

        `field_disc` is the `Field(discriminator=...)` of the owning field, if any."""
        metas: list[Any] = []
        name = alias_hint or self._lookup_alias(t)
        while True:
            if t is JsonValue or t is Any or t is object:
                return Prim("json"), False
            if isinstance(t, typing.TypeAliasType):
                name = name or t.__name__
                t = t.__value__
            elif typing.get_origin(t) is typing.Annotated:
                args = typing.get_args(t)
                t, metas = args[0], metas + list(args[1:])
                name = name or self._lookup_alias(t)
            else:
                break
        discriminator = field_disc or next(
            (m.discriminator for m in metas if isinstance(m, FieldInfo) and m.discriminator), None
        )
        origin = typing.get_origin(t)

        if t is type(None):
            raise GenerationError(f"bare None annotation at {ctx}")
        if t is bool:
            return Prim("bool"), False
        if t is int:
            return Prim("int"), False
        if t is float:
            return Prim("float"), False
        if t is str or t is datetime:
            return Prim("string"), False
        if inspect.isclass(t) and issubclass(t, enum.Enum):
            return self._enum(t), False
        if inspect.isclass(t) and issubclass(t, BaseModel):
            return self._model(t), False
        if origin is typing.Literal:
            return self._literal(typing.get_args(t), name or ctx), False
        if origin in (list, set, frozenset):
            item, nullable = self.convert(typing.get_args(t)[0], ctx + "Item")
            return ListOf(Nullable(item) if nullable else item), False
        if origin is dict:
            key, value = typing.get_args(t)
            if key is not str:
                raise GenerationError(f"non-string dict key at {ctx}")
            item, nullable = self.convert(value, ctx + "Value")
            return MapOf(Nullable(item) if nullable else item), False
        if _is_union(t):
            return self._union(t, discriminator, name, ctx)
        raise GenerationError(f"unsupported annotation {t!r} at {ctx}")

    def _literal(self, values: tuple[Any, ...], name: str) -> TypeRef:
        values = tuple(v.value if isinstance(v, enum.Enum) else v for v in values)
        if len(values) == 1 and isinstance(values[0], (str, bool, int)):
            return Lit(values[0])
        if not all(isinstance(v, str) for v in values):
            raise GenerationError(f"mixed or non-string multi-value Literal for {name}: {values!r}")
        return self._register(EnumDef(pascal(name), sorted(values), "", "vibe (inline Literal)"))

    def _union(self, t: Any, discriminator: str | None, name: str | None, ctx: str) -> tuple[TypeRef, bool]:
        members = list(typing.get_args(t))
        nullable = type(None) in members
        members = [m for m in members if m is not type(None)]
        if len(members) == 1:
            ref, inner_nullable = self.convert(members[0], ctx, name)
            return ref, nullable or inner_nullable
        union_name = NAME_OVERRIDES.get(pascal(name or ctx), pascal(name or ctx))
        if discriminator:
            return self._tagged(union_name, members, discriminator, named=bool(name)), nullable
        stripped = [self._strip(m) for m in members]
        values = _string_values(stripped)
        if values is not None:
            return self._literal(tuple(values), union_name), nullable
        refs: list[TypeRef] = []
        for m in members:
            ref, member_nullable = self.convert(m, union_name + "Member")
            refs.append(Nullable(ref) if member_nullable else ref)
        return self._register(UntaggedDef(union_name, refs, "", "vibe (untagged union)", bool(name))), nullable

    @staticmethod
    def _strip(t: Any) -> Any:
        while True:
            if typing.get_origin(t) is typing.Annotated:
                t = typing.get_args(t)[0]
            elif isinstance(t, typing.TypeAliasType) and t is not JsonValue:
                t = t.__value__
            else:
                return t

    def _tagged(self, union_name: str, members: list[Any], disc: str, named: bool) -> Ref:
        """Register a discriminated union; `disc` names the tag field of every member model."""
        built: list[tuple[str, str]] = []
        tag_wire = ""
        for member in members:
            cls = self._strip(member)
            ref, _ = self.convert(cls, union_name)
            if not (isinstance(ref, Ref) and inspect.isclass(cls) and issubclass(cls, BaseModel)):
                raise GenerationError(f"tagged union {union_name} has a non-model member {member!r}")
            py_name = field_name_of(cls, disc)
            annotation = self._strip(cls.model_fields[py_name].annotation)
            if typing.get_origin(annotation) is not typing.Literal:
                raise GenerationError(f"{cls.__name__}.{disc} is not a Literal tag")
            wire = _alias(cls, py_name)
            if tag_wire and wire != tag_wire:
                raise GenerationError(f"members of {union_name} disagree on the tag key")
            tag_wire = wire
            built.extend((str(_plain(v)), ref.name) for v in typing.get_args(annotation))
        return self._register(UnionDef(union_name, tag_wire, sorted(built), "", "vibe (discriminated union)", named))

    def _enum(self, cls: type[enum.Enum]) -> Ref:
        values = [m.value for m in cls]
        if not all(isinstance(v, str) for v in values):
            raise GenerationError(f"non-string enum {cls.__name__}")
        return self._register(EnumDef(cls.__name__, sorted(values), _doc_of(cls), _source_of(cls)))

    def _model(self, cls: type[BaseModel]) -> Ref:
        if cls in self._models:
            return Ref(self._models[cls])
        name = MODEL_RENAMES.get(cls.__name__, cls.__name__)
        if name in self.defs:
            raise GenerationError(f"two different Vibe types are both named {name}")
        self._models[cls] = name
        cls.model_rebuild()
        fields: list[Field] = []
        for py_name, info in cls.model_fields.items():
            if info.exclude:
                # Never serialized by Vibe, so never on the wire (e.g. RequiredPermission.literal).
                continue
            ref, nullable = self.convert(
                info.annotation, f"{name}{pascal(py_name)}", field_disc=info.discriminator
            )
            fields.append(
                Field(py_name, _alias(cls, py_name), ref, info.is_required(), nullable, info.description or "")
            )
        if cls.model_computed_fields:
            raise GenerationError(f"computed fields on {cls.__name__} are not supported")
        self.defs[name] = ModelDef(name, fields, _doc_of(cls), _source_of(cls))
        return Ref(name)

    def _register(self, definition: Def) -> Ref:
        """Add a synthetic definition, reusing an identical earlier one; names must not clash."""
        existing = self.defs.get(definition.name)
        if existing is not None:
            if _shape(existing) != _shape(definition):
                raise GenerationError(f"conflicting definitions named {definition.name}")
            return Ref(definition.name)
        self.defs[definition.name] = definition
        return Ref(definition.name)

    # ---- post-processing -----------------------------------------------------------------

    def _mark_discriminators(self) -> None:
        for d in self.defs.values():
            if not isinstance(d, UnionDef):
                continue
            for _, model_name in d.members:
                model = self.defs[model_name]
                assert isinstance(model, ModelDef)
                hit = [f for f in model.fields if f.wire == d.tag]
                if len(hit) != 1:
                    raise GenerationError(f"{model_name} has no unique tag field {d.tag!r}")
                # A tagged union cannot be decoded without its tag, so the tag is required even
                # though Pydantic gives it a default; Vibe always emits it.
                hit[0].discriminator = True
                hit[0].required = True

    def _mark_boxes(self) -> None:
        """Box a field when its direct (non-collection) target can reach the model that owns it."""

        def direct(d: Def) -> list[str]:
            if isinstance(d, ModelDef):
                return [n for f in d.fields if not f.discriminator for n in _direct_names(f.type)]
            if isinstance(d, UnionDef):
                return sorted({m for _, m in d.members})
            if isinstance(d, UntaggedDef):
                return [n for t in d.members for n in _direct_names(t)]
            return []

        reach_cache: dict[str, set[str]] = {}

        def reach(name: str) -> set[str]:
            if name in reach_cache:
                return reach_cache[name]
            seen: set[str] = set()
            stack = list(direct(self.defs[name]))
            while stack:
                n = stack.pop()
                if n not in seen:
                    seen.add(n)
                    stack.extend(direct(self.defs[n]))
            reach_cache[name] = seen
            return seen

        for d in self.defs.values():
            if isinstance(d, ModelDef):
                for f in d.fields:
                    f.boxed = any(d.name in reach(n) or n == d.name for n in _direct_names(f.type))


def _direct_names(t: TypeRef, collections: bool = False) -> list[str]:
    """Names reachable from `t`; list and map items count only when `collections` is true."""
    if isinstance(t, Ref):
        return [t.name]
    if isinstance(t, Nullable):
        return _direct_names(t.inner, collections)
    if collections and isinstance(t, (ListOf, MapOf)):
        return [n for n in _direct_names(t.item if isinstance(t, ListOf) else t.value, True)]
    return []


def _plain(value: Any) -> Any:
    """The wire value of a Literal argument (enum members stand for their value)."""
    return value.value if isinstance(value, enum.Enum) else value


def _string_values(members: list[Any]) -> list[str] | None:
    """Union of all string values when every member is a string Literal or string Enum, else None."""
    values: set[str] = set()
    for m in members:
        if typing.get_origin(m) is typing.Literal and all(isinstance(_plain(v), str) for v in typing.get_args(m)):
            values.update(_plain(v) for v in typing.get_args(m))
        elif inspect.isclass(m) and issubclass(m, enum.Enum) and all(isinstance(e.value, str) for e in m):
            values.update(e.value for e in m)
        else:
            return None
    return sorted(values)


def _rename_type(t: TypeRef, rename: dict[str, str]) -> TypeRef:
    """Rewrite references to merged definitions."""
    if isinstance(t, Ref):
        return Ref(rename.get(t.name, t.name))
    if isinstance(t, Nullable):
        return Nullable(_rename_type(t.inner, rename))
    if isinstance(t, ListOf):
        return ListOf(_rename_type(t.item, rename))
    if isinstance(t, MapOf):
        return MapOf(_rename_type(t.value, rename))
    return t


def _is_union(t: Any) -> bool:
    return typing.get_origin(t) in (typing.Union, types.UnionType)


def _shape(d: Def) -> tuple[Any, ...]:
    if isinstance(d, EnumDef):
        return ("enum", tuple(d.values))
    if isinstance(d, UntaggedDef):
        return ("untagged", tuple(d.members))
    if isinstance(d, UnionDef):
        return ("union", d.tag, tuple(d.members))
    return ("model", d.name)


def field_name_of(cls: type[BaseModel], disc: str) -> str:
    """The Python field name for a discriminator given by name or by alias."""
    for py_name, info in cls.model_fields.items():
        if py_name == disc or info.alias == disc:
            return py_name
    raise GenerationError(f"{cls.__name__} has no field {disc!r}")


def _find_field(cls: type[BaseModel], disc: str) -> FieldInfo:
    return cls.model_fields[field_name_of(cls, disc)]


def _alias(cls: type[BaseModel], py_name: str) -> str:
    """Wire key of a field: its explicit alias, else the model's camelCase alias generator."""
    info = cls.model_fields[py_name]
    if info.alias:
        return info.alias
    if cls.model_config.get("alias_generator") is to_camel:
        return to_camel(py_name)
    return py_name


def build_schema(request_map: dict[str, Any], intro: Introspector) -> Schema:
    """Introspect every model named in the method map plus the envelope roots into `intro`."""
    schema = Schema()

    def side(name: str | None) -> str | None:
        return intro.add_root(name) if name else None

    for method, pair in request_map["requests"].items():
        schema.requests[method] = (side(pair["params"]), side(pair["result"]))
    for method, name in request_map["notifications"].items():
        schema.notifications[method] = side(name)
    for method, pair in request_map["serverRequests"].items():
        schema.server_requests[method] = (side(pair["params"]), side(pair["result"]))
    for name in ENVELOPE_ROOTS:
        intro.add_root(name)
    schema.defs = intro.finish()
    return schema
