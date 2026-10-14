/**
 * Applies the UI's lint rules to public component tests outside the UI package directory.
 * Used by the UI lint script; the source config owns the rules.
 */
import uiConfig from "../../../ui/eslint.config.js";

export default [{ ignores: ["eslint.config.mjs"] }, ...uiConfig];
