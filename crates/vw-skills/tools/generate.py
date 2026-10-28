"""Write wire types and replay fixtures from Vibe's installed Pydantic models.

Uses only the installed, version-checked Vibe Python environment.
Run with --write to regenerate or --check to verify committed output.
"""

from __future__ import annotations

import argparse
import copy
import importlib
import importlib.metadata
import json
from pathlib import Path
import re
import subprocess

from pydantic import TypeAdapter

CRATE = Path(__file__).resolve().parents[1]
REPO = CRATE.parents[1]
TAG = "3.0.2"
MODULES = ("protocol", "models", "config")
METHODS = {
    "list": ("SkillsListParams", "SkillsListResponse"),
    "installed": ("SkillsInstalledParams", "SkillsInstalledResponse"),
    "catalog": ("SkillsCatalogParams", "SkillsCatalogResponse"),
    "versions": ("SkillsVersionsParams", "SkillsVersionsResponse"),
    "updates": ("SkillsUpdatesParams", "SkillsUpdatesResponse"),
    "detail": ("SkillsDetailParams", "SkillsDetailResponse"),
    "import": ("SkillsImportParams", "RuntimeMutationResponse"),
    "setVersion": ("SkillsSetVersionParams", "RuntimeMutationResponse"),
    "setLatest": ("SkillsSetLatestParams", "RuntimeMutationResponse"),
    "setAlias": ("SkillsSetAliasParams", "RuntimeMutationResponse"),
    "remove": ("SkillsRemoveParams", "RuntimeMutationResponse"),
    "setEnabled": ("SkillsSetEnabledParams", "RuntimeMutationResponse"),
    "convertLocal": ("SkillsConvertLocalParams", "SkillsConvertResponse"),
}


def rust_name(name):
    return name.replace("MCP", "Mcp").replace("TTS", "Tts")


def variant(value):
    return "".join(word.capitalize() for word in value.split("_"))


def snake(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def rust_type(schema):
    if "$ref" in schema:
        name = schema["$ref"].rsplit("/", 1)[1]
        return "serde_json::Value" if name == "JsonValue" else rust_name(name)
    if "anyOf" in schema:
        items = [item for item in schema["anyOf"] if item.get("type") != "null"]
        if len(items) != 1:
            raise ValueError(f"Unsupported union: {schema}")
        return f"Option<{rust_type(items[0])}>"
    match schema.get("type"):
        case "string":
            return "String"
        case "integer":
            return "i64"
        case "number":
            return "f64"
        case "boolean":
            return "bool"
        case "array":
            return f"Vec<{rust_type(schema['items'])}>"
        case "object":
            return f"std::collections::BTreeMap<String, {rust_type(schema['additionalProperties'])}>"
        case _:
            raise ValueError(f"Unsupported type: {schema}")


def literal(value, schema):
    ty = rust_type(schema)
    if value is None:
        return "None" if ty.startswith("Option<") else "serde_json::Value::Null"
    if "$ref" in schema:
        return f"{ty}::{variant(value)}"
    if isinstance(value, str):
        return f"{json.dumps(value)}.to_owned()"
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, list):
        return "vec![" + ", ".join(literal(v, schema["items"]) for v in value) + "]"
    return repr(value)


def render(name, schema, classes):
    ty = rust_name(name)
    if "enum" in schema or "const" in schema:
        lines = [f"/// Vibe's `{name}` wire values.", "#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]", f"pub enum {ty} {{"]
        for value in schema.get("enum", [schema.get("const")]):
            lines += [f"    /// Wire value `{value}`.", f"    #[serde(rename = {json.dumps(value)})]", f"    {variant(value)},"]
        return "\n".join(lines + ["}", ""])
    cls = classes[name]
    lines = [f"/// Vibe's `{name}` model; unknown extra fields are ignored.", "#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]", f"pub struct {ty} {{"]
    if sum(prop.get("type") == "boolean" for prop in schema["properties"].values()) > 3:
        lines.insert(2, "#[allow(clippy::struct_excessive_bools)] // Independent upstream wire flags.")
    defaults = []
    for alias, prop in schema["properties"].items():
        field = next(key for key, info in cls.model_fields.items() if (info.alias or key) == alias)
        lines += [f"    /// Vibe field `{alias}`.", f"    #[serde(rename = {json.dumps(alias)})]"]
        if alias not in schema.get("required", []):
            info = cls.model_fields[field]
            default = TypeAdapter(info.annotation).dump_python(info.get_default(call_default_factory=True), mode="json")
            if default in (None, False, "", 0) or default == [] or default == {}:
                lines += ["    #[serde(default)]"]
            else:
                fn = f"default_{snake(ty)}_{field}"
                lines += [f'    #[serde(default = "{fn}")]']
                defaults += [f"fn {fn}() -> {rust_type(prop)} {{", f"    {literal(default, prop)}", "}", ""]
        lines += [f"    pub {field}: {rust_type(prop)},"]
    lines += ["}", "", *defaults]
    # This model has an after-validator in Vibe: blank labels use the stable id.
    if name == "MCPSourceSummary":
        lines.insert(2, '#[serde(from = "McpSourceSummaryWire")]')
        body = "\n".join(lines[4:lines.index("}")])
        body = body.replace("    pub ", "    ")
        lines += ["#[derive(serde::Deserialize)]", "struct McpSourceSummaryWire {", body, "}", "", "impl From<McpSourceSummaryWire> for McpSourceSummary {", "    fn from(value: McpSourceSummaryWire) -> Self {", "        Self {"]
        for field in cls.model_fields:
            val = "if value.display_name.is_empty() { value.name.clone() } else { value.display_name }" if field == "display_name" else f"value.{field}"
            # Clone the name because the display-name validator also reads it.
            if field == "name":
                val = "value.name.clone()"
            lines += [f"            {field}: {val},"]
        lines += ["        }", "    }", "}", ""]
    return "\n".join(lines)


def example(schema, definitions):
    if "default" in schema:
        return schema["default"]
    if "$ref" in schema:
        return example(definitions[schema["$ref"].rsplit("/", 1)[1]], definitions)
    if "const" in schema:
        return schema["const"]
    if "enum" in schema:
        return schema["enum"][0]
    if "anyOf" in schema:
        return None
    match schema.get("type"):
        case "object":
            return {key: example(prop, definitions) for key, prop in schema.get("properties", {}).items() if key in schema.get("required", [])}
        case "array":
            return []
        case "string":
            return "example"
        case "integer":
            return 0
        case "number":
            return 0.0
        case "boolean":
            return False
        case _:
            raise ValueError(schema)


def generate():
    modules = [importlib.import_module(f"vibe.app_server.{name}") for name in MODULES]
    classes = {name: value for module in modules for name, value in vars(module).items() if isinstance(value, type) and hasattr(value, "model_json_schema")}
    roots = {name for names in METHODS.values() for name in names} | {"ProtocolError"}
    schemas = {name: classes[name].model_json_schema() for name in sorted(roots)}
    definitions = {name: definition for schema in schemas.values() for name, definition in schema.get("$defs", {}).items()}
    definitions.update({name: {k: v for k, v in schema.items() if k != "$defs"} for name, schema in schemas.items()})
    definitions = copy.deepcopy(definitions)
    definitions.pop("JsonValue", None)
    for name, schema in list(definitions.items()):
        for alias, prop in schema.get("properties", {}).items():
            candidate = next((item for item in prop.get("anyOf", []) if "enum" in item), prop)
            if "enum" not in candidate:
                continue
            enum_name = {("SkillSummary", "source"): "SkillSource", ("SkillSummary", "scope"): "SkillScope", ("ModelConfigView", "imageDelivery"): "ImageDelivery"}.get((name, alias), name + alias[0].upper() + alias[1:])
            definitions[enum_name] = copy.deepcopy(candidate)
            candidate.pop("enum")
            candidate.pop("type", None)
            candidate["$ref"] = f"#/$defs/{enum_name}"
    # Named Literals have no Python model class but are enum definitions in schema.
    groups = {"config": set(modules[2].__dict__) | {"ThinkingLevel", "AudioClient", "TranscriptionEncoding", "SpeechOutputFormat", "ImageDelivery"}, "errors": {"ProtocolError", "ProtocolErrorCode"}, "params": roots - {"ProtocolError", "RuntimeMutationResponse"}, "skills": {"SkillSummary", "RegistryRefView", "SkillCatalogEntry", "SkillVersionView", "SkillUpdateView", "SkillDetailView", "SkillSource", "SkillScope"}}
    output = {}
    for group in ["config", "errors", "params", "skills", "runtime"]:
        names = [name for name in definitions if (name in groups.get(group, set()) if group != "runtime" else not any(name in names for names in groups.values()))]
        contents = f"//! GENERATED by tools/generate.py from Vibe {TAG}; do not edit.\n//! Serde wire models for `SkillsClient`; no workspace dependencies.\n\n"
        def refs(value):
            if isinstance(value, dict):
                return ({value["$ref"].rsplit("/", 1)[1]} if "$ref" in value else set()) | set().union(*(refs(v) for v in value.values()))
            if isinstance(value, list):
                return set().union(*(refs(v) for v in value))
            return set()
        imports = set().union(*(refs(definitions[name]) for name in names)) - set(names) - {"JsonValue"}
        if group == "params":
            imports.add("RuntimeMutationResponse")
        if imports:
            contents += "use super::{" + ", ".join(rust_name(name) for name in sorted(imports)) + "};\n\n"
        contents += "\n".join(render(name, definitions[name], classes) for name in sorted(names))
        if group == "params":
            for method in ("Import", "SetVersion", "SetLatest", "SetAlias", "Remove", "SetEnabled"):
                contents += f"\n/// Full runtime result of `skills/{method[0].lower() + method[1:]}`.\npub type Skills{method}Response = RuntimeMutationResponse;\n"
        output[CRATE / f"src/wire/{group}.rs"] = contents
    protocol = modules[0]
    skill = {"name": "review", "description": "Review code", "prompt": "Review the changes.", "source": "registry", "registry": {"skillId": "skill-review", "version": 2}}
    project = {**skill, "description": "Project review", "scope": "project", "registry": {"skillId": "skill-project-review", "version": 3}}
    disabled = {"name": "draft", "description": "Write a draft", "prompt": "Draft text.", "enabled": False}
    builtin = {"name": "vibe", "description": "Vibe guidance", "prompt": "Use Vibe.", "source": "builtin", "scope": "builtin", "locked": True}
    plugin = {"name": "plugin:notes", "description": "Plugin notes", "prompt": "Read notes.", "source": "plugin", "locked": True}
    catalog = {"skills": [{"name": "review", "skillId": "skill-project-review", "description": "Catalog review", "latestVersion": 4}, {"name": "catalog-only", "skillId": "skill-catalog", "description": "Available to import", "latestVersion": 1}], "updates": {"review": 4}, "loaded": True, "projectAvailable": True}
    raw_runtime = example(schemas["RuntimeMutationResponse"]["$defs"]["RuntimeSnapshot"], definitions)
    raw_runtime.update(skills=[project, builtin, plugin], issues=[{"file": "/workspace/.vibe/skills/broken/SKILL.md", "message": "Missing description"}])
    runtime = protocol.RuntimeSnapshot.model_validate(raw_runtime).model_dump(mode="json", by_alias=True)
    results = {"list": {"skills": [project, builtin, plugin]}, "installed": {"skills": [skill, disabled, project, plugin]}, "catalog": catalog, "versions": {"versions": [{"version": 4, "aliases": ["latest", "stable"]}, {"version": 3}]}, "updates": {"updates": [{"name": "review", "currentVersion": 3, "latestVersion": 4}]}, "detail": {"detail": {"name": "review", "skillId": "skill-project-review", "version": 4, "body": "# Review"}}, **{method: {"runtime": runtime} for method in METHODS if method not in ("list", "installed", "catalog", "versions", "updates", "detail")}}
    results["convertLocal"]["converted"] = True
    params = {"sessionId": "session-skills", "name": "review", "skillId": "skill-project-review", "version": 4, "alias": "stable", "scope": "project", "enabled": False}
    for method, (request_name, response_name) in METHODS.items():
        request_class, response_class = classes[request_name], classes[response_name]
        fields = {info.alias for info in request_class.model_fields.values()}
        request = request_class.model_validate({key: value for key, value in params.items() if key in fields}).model_dump(mode="json", by_alias=True)
        response = response_class.model_validate(results[method]).model_dump(mode="json", by_alias=True)
        output[CRATE / f"tests/fixtures/{method}.json"] = json.dumps({"method": f"skills/{method}", "params": request, "result": response}, indent=2) + "\n"
    errors = [("import", "invalid_params", "no authenticated Mistral endpoint available"), ("setEnabled", "invalid_params", "'review' is fixed by configuration and cannot be toggled here"), ("remove", "conflict", "Session is busy running turn turn-test"), ("import", "forbidden", "Access denied"), ("list", "method_not_found", "Unknown method"), ("list", "internal_error", "Failed to read skills")]
    output[CRATE / "tests/fixtures/errors.json"] = json.dumps([{"method": f"skills/{method}", "error": protocol.ProtocolError(code=code, message=message).model_dump(mode="json", by_alias=True)} for method, code, message in errors], indent=2) + "\n"
    wire_schemas = {name: definition for schema in schemas.values() for name, definition in schema.get("$defs", {}).items()}
    wire_schemas.update({name: {k: v for k, v in schema.items() if k != "$defs"} for name, schema in schemas.items()})
    for name, schema in wire_schemas.items():
        output[CRATE / f"tests/fixtures/schema/{name}.json"] = json.dumps(schema, indent=2, sort_keys=True) + "\n"
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()
    installed = importlib.metadata.version("mistral-vibe")
    if installed != TAG:
        raise SystemExit(f"Expected installed Vibe {TAG}, found {installed}")
    output = generate()
    failed = []
    for path, contents in output.items():
        if path.suffix == ".rs":
            contents = subprocess.check_output(["rustfmt", "--edition", "2024"], input=contents.encode()).decode()
        if args.write:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents)
        elif not path.exists() or path.read_text() != contents:
            failed.append(str(path.relative_to(REPO)))
    if failed:
        raise SystemExit("Generated files differ: " + ", ".join(failed))
    print(f"Vibe {TAG}: {len(output)} generated files {'written' if args.write else 'match'}")


if __name__ == "__main__":
    main()
