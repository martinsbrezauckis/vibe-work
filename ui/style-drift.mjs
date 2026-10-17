/**
 * Checks Settings, Vault, project, browser and app-shell controls and typography against shared UI styles.
 * Depends on: TypeScript's JSX parser and the local feature/style sources.
 * Called by: UI lint and the style guard tests. The exception table is exact by file and class.
 */
import { readFileSync, existsSync, readdirSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const uiRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = join(uiRoot, "src");
const featureRoot = join(sourceRoot, "features");
const styleRoot = join(sourceRoot, "styles");
const cssNames = [
  "settings-page.css",
  "settings.css",
  "vault-page.css",
  "vault-actions.css",
  "updates.css",
  "setup.css",
  "projects.css",
  "preview.css",
  "shell.css",
  "overlays.css",
  "responsive.css",
  "feel.css",
  "panel.css",
  "sessions.css",
  "search.css",
];
const sharedButtonClasses = new Set(["tb-btn", "switch", "ext-link", "btn-primary"]);
// These setup primitives predate the Settings/Vault control kit and also render in KeySettings.
const setupButtonClasses = new Set(["btn-secondary", "key-link"]);
const exactControlClasses = new Map([
  ["settings/SettingsNav.tsx", new Set(["scope-chip", "set-result", "set-cat"])],
  ["settings/SettingsPage.tsx", new Set(["set-hit"])],
  ["secrets/VaultLockControls.tsx", new Set(["sb-vault"])],
  ["preview/Toolbar.tsx", new Set(["browser-menu-item"])],
  ["shell/TitleBar.tsx", new Set(["search", "dbg"])],
  ["shell/StatusBar.tsx", new Set(["sb-usage"])],
  // Shell navigation and disclosure controls retain their keyboard/layout contracts.
  ["shell/CommandPalette.tsx", new Set(["pal-more-action"])],
  ["shell/Content.tsx", new Set(["pbtn"])],
  ["shell/LeftRunning.tsx", new Set(["sb-usage"])],
  ["shell/Rail.tsx", new Set(["rbtn"])],
  ["shell/SessionList.tsx", new Set(["needs-row", "sec-btn", "more-btn", "rbtn-wide"])],
  ["shell/SidePanel.tsx", new Set(["ptab", "ptab-x"])],
]);
const controlModifiers = new Set(["primary", "danger", "sm"]);
const exactControlExtras = new Map([
  ["secrets/VaultItemActions.tsx", new Set(["vault-actions-trigger"])],
  ["secrets/McpLabel.tsx", new Set(["mcp-info-btn"])],
  ["shell/StatusBar.tsx", new Set(["sb-rate"])],
  ["shell/LeftRunning.tsx", new Set(["sb-left"])],
]);
const exactPillClasses = new Map([["shell/SidePanel.tsx", new Set(["ptab-pill"])]]);
const pillName = /(?:^|-)(?:badge|chip|pill|tag)(?:$|-)/;
const fontName = /^(?:font|font-family|font-size|font-weight)$/i;
const typographyToken = /^var\(--(?:font-(?:ui|mono)|text-[a-z0-9-]+|weight-[a-z0-9-]+)\)$/;
const sharedTypography = new Set(
  [
    ...readFileSync(join(styleRoot, "tokens.css"), "utf8").matchAll(
      /(--(?:font-(?:ui|mono)|text-[a-z0-9-]+|weight-[a-z0-9-]+))\s*:/g,
    ),
  ].map((match) => `var(${match[1]})`),
);

function classValues(attribute) {
  if (!attribute || !ts.isJsxAttribute(attribute) || !attribute.initializer) return null;
  if (ts.isStringLiteral(attribute.initializer)) return [attribute.initializer.text];
  if (!ts.isJsxExpression(attribute.initializer) || !attribute.initializer.expression) return null;
  const values = [];
  function collect(node) {
    if (ts.isStringLiteralLike(node)) values.push(node.text);
    else if (ts.isConditionalExpression(node)) {
      return collect(node.whenTrue) && collect(node.whenFalse);
    } else if (ts.isParenthesizedExpression(node)) return collect(node.expression);
    else return false;
    return true;
  }
  return collect(attribute.initializer.expression) ? values : null;
}

function jsxName(node) {
  return ts.isIdentifier(node) ? node.text : "";
}

// Literal geometry branches are safe; an identifier, computed key or style spread is opaque.
function inspectInlineStyle(expression, report, tree) {
  if (ts.isParenthesizedExpression(expression) || ts.isAsExpression(expression))
    return inspectInlineStyle(expression.expression, report, tree);
  if (ts.isConditionalExpression(expression)) {
    const trueBranch = inspectInlineStyle(expression.whenTrue, report, tree);
    const falseBranch = inspectInlineStyle(expression.whenFalse, report, tree);
    return trueBranch && falseBranch;
  }
  if (!ts.isObjectLiteralExpression(expression)) return false;
  for (const property of expression.properties) {
    if (ts.isSpreadAssignment(property)) {
      if (!inspectInlineStyle(property.expression, report, tree))
        report(property, "dynamic inline styles bypass shared typography tokens");
      continue;
    }
    if (!ts.isPropertyAssignment(property) || ts.isComputedPropertyName(property.name)) {
      report(property, "dynamic inline styles bypass shared typography tokens");
      continue;
    }
    const name = property.name.getText(tree).replace(/^[\x27\x22]|[\x27\x22]$/g, "");
    if (
      fontName.test(name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)) ||
      /^--(?:font|text|weight)(?:-|$)/.test(name)
    )
      report(property, `inline ${name} bypasses shared typography tokens`);
  }
  return true;
}

function scanTsx(file, source, scopeName) {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const findings = [];
  function report(node, message) {
    const { line } = tree.getLineAndCharacterOfPosition(node.getStart(tree));
    findings.push(`${file}:${line + 1}: ${message}`);
  }
  function visit(node) {
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const tag = jsxName(node.tagName);
      const classAttribute = node.attributes.properties.find((a) => a.name?.text === "className");
      const classes = classValues(classAttribute);
      const allowed = exactControlClasses.get(scopeName) ?? new Set();
      if (tag === "select") report(node, "raw <select> needs the shared Selector component");
      const typeAttribute = node.attributes.properties.find((a) => a.name?.text === "type");
      if (tag === "Segmented" || (tag === "input" && classValues(typeAttribute)?.includes("radio")))
        report(node, "pick-one controls need the shared Selector component");
      if (tag === "button" || tag === "a") {
        if (
          classes === null ||
          classes.length === 0 ||
          classes.some((entry) => {
            const names = entry.split(/\s+/).filter(Boolean);
            const base = names.some(
              (name) =>
                sharedButtonClasses.has(name) ||
                allowed.has(name) ||
                (scopeName.startsWith("setup/") && setupButtonClasses.has(name)),
            );
            const extras = exactControlExtras.get(scopeName);
            return (
              !base ||
              names.some(
                (name) =>
                  !sharedButtonClasses.has(name) &&
                  !allowed.has(name) &&
                  !controlModifiers.has(name) &&
                  !extras?.has(name) &&
                  !(scopeName.startsWith("setup/") && setupButtonClasses.has(name)),
              )
            );
          })
        )
          report(
            node,
            `<${tag}> needs Button, IconButton, Selector, a shared control class, or an exact navigation/disclosure exception`,
          );
      }
      if (
        tag === "span" &&
        classes?.some((entry) =>
          entry
            .split(/\s+/)
            .some((name) => pillName.test(name) && !exactPillClasses.get(scopeName)?.has(name)),
        )
      )
        report(node, "ad-hoc pill/tag needs Badge or a documented exact exception");
      const styleAttribute = node.attributes.properties.find((a) => a.name?.text === "style");
      if (
        styleAttribute &&
        ts.isJsxAttribute(styleAttribute) &&
        styleAttribute.initializer &&
        ts.isJsxExpression(styleAttribute.initializer) &&
        styleAttribute.initializer.expression
      ) {
        const expression = styleAttribute.initializer.expression;
        if (!inspectInlineStyle(expression, report, tree))
          report(
            styleAttribute,
            "referenced inline styles need CSS or a documented geometry exception",
          );
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(tree);
  return findings;
}

function scanCss(file, source) {
  const clean = source.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, " "));
  const findings = [];
  const declaration = /(?:^|[;{}])\s*(font(?:-family|-size|-weight)?)\s*:\s*([^;{}]+)/gm;
  for (const match of clean.matchAll(declaration)) {
    const property = match[1];
    const value = match[2].trim();
    if (value === "inherit" || (typographyToken.test(value) && sharedTypography.has(value)))
      continue;
    const line = clean.slice(0, match.index + match[0].indexOf(property)).split("\n").length;
    findings.push(
      `${file}:${line}: ${property}: ${value} needs inherit or a shared typography token`,
    );
  }
  for (const match of clean.matchAll(
    /(?:^|[;{}])\s*(--(?:font-[a-z0-9-]+|text-[a-z0-9-]+|weight-[a-z0-9-]+))\s*:/gm,
  )) {
    const line = clean.slice(0, match.index + match[0].indexOf(match[1])).split("\n").length;
    findings.push(
      `${file}:${line}: ${match[1]} must be defined in shared tokens.css, not feature CSS`,
    );
  }
  return findings;
}

function localCssImports(file, source) {
  const imports = [];
  const pattern = /(?:@import\s+["']|\bimport\s+["'])(\.[^"']+\.css)["']/g;
  for (const match of source.matchAll(pattern)) {
    const target = resolve(dirname(file), match[1]);
    if (existsSync(target) && target.startsWith(`${sourceRoot}/`)) imports.push(target);
  }
  return imports;
}

function filesWithExtension(directory, extension) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...filesWithExtension(path, extension));
    else if (entry.isFile() && entry.name.endsWith(extension)) found.push(path);
  }
  return found;
}

function componentImports(file, source) {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const used = new Set();
  function visit(node) {
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node))
      used.add(jsxName(node.tagName));
    ts.forEachChild(node, visit);
  }
  visit(tree);
  const imports = [];
  for (const statement of tree.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier))
      continue;
    const specifier = statement.moduleSpecifier.text;
    if (!specifier.startsWith(".")) continue;
    const clause = statement.importClause;
    const names = [clause?.name?.text];
    if (clause?.namedBindings && ts.isNamedImports(clause.namedBindings)) {
      names.push(...clause.namedBindings.elements.map((e) => e.name.text));
    }
    if (!names.some((name) => name && used.has(name))) continue;
    const target = resolve(dirname(file), specifier);
    const tsx = target.endsWith(".tsx") ? target : `${target}.tsx`;
    if (existsSync(tsx) && tsx.startsWith(`${featureRoot}/`)) imports.push(tsx);
  }
  return imports;
}

/** Scan Settings/project/browser JSX import graphs, all shell modules and their CSS. */
export function checkStyleDrift(options = {}) {
  const read = options.read ?? ((path) => readFileSync(path, "utf8"));
  const files = options.files ?? [
    join(featureRoot, "settings", "SettingsPage.tsx"),
    join(featureRoot, "settings", "SettingsNav.tsx"),
    join(featureRoot, "projects", "ProjectSettings.tsx"),
    join(featureRoot, "preview", "Toolbar.tsx"),
    ...filesWithExtension(join(featureRoot, "shell"), ".tsx"),
  ];
  const cssFiles = options.cssFiles ?? cssNames.map((name) => join(styleRoot, name));
  const seen = new Set();
  const pending = [...files];
  const findings = [];
  const stylePending = [...cssFiles];
  if (options.cssFiles === undefined) stylePending.push(...filesWithExtension(featureRoot, ".css"));
  while (pending.length > 0) {
    const file = pending.pop();
    if (!file || seen.has(file)) continue;
    seen.add(file);
    const source = read(file);
    const name = relative(featureRoot, file).replaceAll("\\", "/");
    findings.push(...scanTsx(relative(uiRoot, file), source, name));
    if (options.followImports !== false) {
      // Shell entry files are complete as a group. Cross-feature children belong to their own screens.
      if (!name.startsWith("shell/")) pending.push(...componentImports(file, source));
      stylePending.push(...localCssImports(file, source));
    }
  }
  const styled = new Set();
  while (stylePending.length > 0) {
    const file = stylePending.pop();
    if (!file || styled.has(file)) continue;
    styled.add(file);
    const source = read(file);
    findings.push(...scanCss(relative(uiRoot, file), source));
    if (options.cssFiles === undefined) stylePending.push(...localCssImports(file, source));
  }
  return findings.sort();
}

/**
 * Every `var(--name)` without a fallback names a custom property that some UI source defines. An undefined one drops
 * the declaration to the browser default: a native list drawn white behind the dark theme's light text. Names built
 * at run time (a prefix ending in "-") are not checked.
 * @param {string} root the folder to scan (default: ui/src)
 * @returns {string[]} one finding per undefined name, with the first file that uses it
 */
export function undefinedCustomProperties(root = sourceRoot) {
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (/\.(css|tsx?)$/.test(entry.name)) files.push(path);
    }
  };
  walk(root);
  const defined = new Set();
  const used = new Map();
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    for (const m of text.matchAll(/(--[\w-]+)\s*:/g)) defined.add(m[1]);
    for (const m of text.matchAll(/["'`](--[\w-]+)["'`]\s*[:,\]]/g)) defined.add(m[1]);
    for (const m of text.matchAll(/setProperty\(\s*["'`](--[\w-]+)/g)) defined.add(m[1]);
    for (const m of text.matchAll(/var\((--[\w-]+)\s*(,)?/g)) {
      if (m[2] !== undefined || m[1].endsWith("-")) continue;
      if (!used.has(m[1])) used.set(m[1], relative(uiRoot, file).replaceAll("\\", "/"));
    }
  }
  return [...used]
    .filter(([name]) => !defined.has(name))
    .map(([name, file]) => `${file}: var(${name}) is defined nowhere in the UI sources`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const findings = [...checkStyleDrift(), ...undefinedCustomProperties()];
  if (findings.length > 0) {
    process.stderr.write(`${findings.join("\n")}\nstyle-drift: ${findings.length} violation(s)\n`);
    process.exitCode = 1;
  } else process.stdout.write("style-drift: PASS\n");
}
