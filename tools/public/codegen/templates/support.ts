/**
 * Shared helper types for the generated protocol types.
 *
 * Used by: every generated area file. `JsonValue` is Vibe's own `JsonValue` (any JSON value).
 * `Unrecognized<Tag>` is the extra member of every discriminated union: a payload whose tag this
 * generated version does not know (a newer Vibe). Its tag property is typed as an opaque brand
 * (not a string) so `switch (x.tag)` over the known literals narrows the known members exactly and
 * leaves `Unrecognized` for the `default:` branch, where it must be handled explicitly. At runtime
 * the tag is of course whatever string Vibe sent; the brand only keeps it out of the known cases.
 */

/** Any JSON value. */
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

/** Opaque brand typing the tag of an unrecognised union member (never a real runtime value). */
export declare const unrecognizedTag: unique symbol;

/** A union member whose `Tag` value is not known to this generated version; carries raw JSON. */
export type Unrecognized<Tag extends string> = {
  readonly [K in Tag]: typeof unrecognizedTag;
} & {
  readonly [key: string]: unknown;
};
