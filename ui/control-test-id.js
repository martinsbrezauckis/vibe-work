/** Enforces stable control hooks in JSX. Used by UI ESLint; native elements and shared controls are checked. */
const native = new Set([
  "button",
  "a",
  "input",
  "select",
  "textarea",
  "summary",
  "dialog",
  "option",
]);
const shared = new Set([
  "Button",
  "IconButton",
  "Switch",
  "Tabs",
  "Tab",
  "TabClose",
  "LinkButton",
  "RowDisclosure",
  "CopyButton",
  "Selector",
  "SecretField",
  "NewPasswordField",
  "RememberChoice",
  "ExternalLink",
  "Tool",
  "ScopePicker",
]);
const roles =
  /^(button|tab|dialog|menuitem|menuitemcheckbox|menuitemradio|option|switch|checkbox|radio|combobox|listbox|slider|spinbutton)$/;
const hookProps = new Set([
  "CopyButton",
  "Selector",
  "SecretField",
  "NewPasswordField",
  "RememberChoice",
  "ExternalLink",
  "Tool",
  "ScopePicker",
]);
const format = /^[a-z][a-z0-9-]*(\.[a-z0-9-]+)+$/;

export default {
  meta: {
    type: "problem",
    schema: [],
    messages: {
      missing: "Interactive {{name}} needs a stable {{prop}} (<area>.<control>).",
      format: "Control {{name}} has an invalid test id; use <area>.<control> in kebab/dot form.",
      translated:
        "Control {{name}} needs a stable test id independent of visible text and item ids.",
      item: "Repeated control {{name}} needs a separate data-item-id on the control or its row.",
    },
  },
  create(context) {
    const aliases = new Map();
    return {
      ImportDeclaration(node) {
        for (const specifier of node.specifiers) {
          if (
            specifier.type === "ImportSpecifier" &&
            (shared.has(specifier.imported.name) || specifier.imported.name === "Badge")
          )
            aliases.set(specifier.local.name, specifier.imported.name);
        }
      },
      JSXOpeningElement(node) {
        if (!["JSXIdentifier", "JSXMemberExpression"].includes(node.name.type)) return;
        const tag = node.name.name ?? node.name.property.name;
        const control = aliases.get(tag) ?? tag;
        const attrs = node.attributes.filter((a) => a.type === "JSXAttribute");
        const get = (name) => attrs.find((a) => a.name.name === name);
        const role = get("role")?.value?.value;
        const events = attrs.some((a) =>
          /^on(Click|DoubleClick|Change|KeyDown|PointerDown|ContextMenu|MouseDown|MouseUp|TouchStart|DragStart|Drop|KeyUp|KeyPress)$/.test(
            a.name.name,
          ),
        );
        if (
          !native.has(tag) &&
          !shared.has(control) &&
          !(control === "Badge" && get("as")?.value?.value === "button") &&
          !(
            tag === tag.toLowerCase() &&
            (events || roles.test(role ?? "") || get("tabIndex") || get("contentEditable"))
          )
        )
          return;
        const prop = hookProps.has(control) ? "testId" : "data-testid";
        const label = get("aria-label")?.value?.value ?? get("label")?.value?.value;
        const name = `<${tag}>${typeof label === "string" ? ` (${label})` : ""}`;
        const hook = get(prop);
        const expression =
          hook?.value?.type === "JSXExpressionContainer" ? hook.value.expression : null;
        const literal =
          hook?.value?.type === "Literal"
            ? hook.value.value
            : expression?.type === "Literal"
              ? expression.value
              : null;
        if (
          !hook?.value ||
          (hook.value.type === "JSXExpressionContainer" &&
            ["JSXEmptyExpression", "Literal"].includes(hook.value.expression.type) &&
            !hook.value.expression.value)
        )
          context.report({ node, messageId: "missing", data: { name, prop } });
        else if (literal !== null && (typeof literal !== "string" || !format.test(literal)))
          context.report({ node: hook, messageId: "format", data: { name } });
        else if (
          expression &&
          /\b(label|text|title|children|itemId)\b|\.name\b|\bterm\s*\(/.test(
            context.sourceCode.getText(expression),
          )
        )
          context.report({ node: hook, messageId: "translated", data: { name } });
        let parent = node;
        let item = Boolean(get("data-item-id"));
        while ((parent = parent.parent)) {
          if (parent.type === "JSXElement")
            item ||= parent.openingElement.attributes.some(
              (a) => a.type === "JSXAttribute" && a.name.name === "data-item-id",
            );
          if (parent.type === "ArrowFunctionExpression" || parent.type === "FunctionExpression") {
            if (
              parent.parent?.type === "CallExpression" &&
              parent.parent.callee.property?.name === "map" &&
              !item
            )
              context.report({ node, messageId: "item", data: { name } });
            break;
          }
        }
      },
    };
  },
};
