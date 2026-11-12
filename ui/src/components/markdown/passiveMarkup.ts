/** Converts renderer XML into React elements. Only passive SVG/MathML tags and attributes survive; never inserts HTML. */
import { createElement, type ReactNode } from "react";
import { SaxesParser } from "saxes";

interface XmlElement {
  name: string;
  attributes: Record<string, string>;
  children: (XmlElement | string)[];
}

// A browser DOMParser can process SVG styles even in a detached XML document. This parser has
// no DOM or resource loader: upstream font imports and hostile tags are just strings to discard.
function parseXml(xml: string): XmlElement {
  const parser = new SaxesParser();
  const stack: XmlElement[] = [];
  let root: XmlElement | undefined;
  let count = 0;
  parser.on("opentag", (tag) => {
    if (++count > 12_000 || stack.length > 80) throw new Error("Drawing too large");
    const node: XmlElement = { name: tag.name, attributes: tag.attributes, children: [] };
    const parent = stack.at(-1);
    if (parent) parent.children.push(node);
    else root = node;
    stack.push(node);
  });
  parser.on("text", (text) => {
    stack.at(-1)?.children.push(text);
  });
  parser.on("closetag", () => {
    stack.pop();
  });
  parser.write(xml).close();
  if (!root) throw new Error("Missing drawing");
  return root;
}

const svgTags = new Set(
  "svg g path rect circle ellipse line polyline polygon text tspan defs marker title desc".split(
    " ",
  ),
);
const mathTags = new Set(
  "math semantics annotation mrow mi mn mo mtext mspace ms mfrac msqrt mroot mstyle merror mpadded mphantom mfenced menclose msub msup msubsup munder mover munderover mmultiscripts mprescripts none mtable mtr mtd".split(
    " ",
  ),
);
const svgAttrs = new Set(
  "viewBox width height x y x1 x2 y1 y2 dx dy cx cy r rx ry d points transform fill stroke stroke-width stroke-dasharray stroke-linecap stroke-linejoin opacity fill-opacity stroke-opacity font-size font-weight font-style text-anchor dominant-baseline markerWidth markerHeight refX refY orient markerUnits preserveAspectRatio".split(
    " ",
  ),
);
const mathAttrs = new Set(
  "display mathvariant mathsize mathcolor mathbackground stretchy symmetric largeop movablelimits fence separator accent accentunder form lspace rspace minsize maxsize width height depth voffset linethickness bevelled notation columnalign rowalign columnspacing rowspacing columnlines rowlines equalrows equalcolumns columnspan rowspan frame framespacing side scriptlevel displaystyle xmlns encoding".split(
    " ",
  ),
);
const camel: Record<string, string> = {
  "stroke-width": "strokeWidth",
  "stroke-dasharray": "strokeDasharray",
  "stroke-linecap": "strokeLinecap",
  "stroke-linejoin": "strokeLinejoin",
  "fill-opacity": "fillOpacity",
  "stroke-opacity": "strokeOpacity",
  "font-size": "fontSize",
  "font-weight": "fontWeight",
  "font-style": "fontStyle",
  "text-anchor": "textAnchor",
  "dominant-baseline": "dominantBaseline",
  "marker-end": "markerEnd",
  "marker-start": "markerStart",
};

function hasControls(value: string): boolean {
  for (let i = 0; i < value.length; i++) if (value.charCodeAt(i) < 32) return true;
  return false;
}

/** Parse without a DOM or resource loader, then rebuild only the allowed drawing/math vocabulary. */
export function passiveMarkup(
  xml: string,
  kind: "svg" | "math",
  prefix: string,
  label: string,
): ReactNode {
  if (xml.length > 1_000_000 || /<!DOCTYPE|<!ENTITY/i.test(xml)) throw new Error("Invalid drawing");
  const doc = parseXml(xml);
  // KaTeX's outer span is packaging; the only allowed root is its MathML element.
  const root =
    kind === "math" && doc.name === "span"
      ? doc.children.find(
          (node): node is XmlElement => typeof node !== "string" && node.name === "math",
        )
      : doc;
  if (!root || root.name !== kind) throw new Error("Missing drawing");
  const tags = kind === "svg" ? svgTags : mathTags;
  const attrs = kind === "svg" ? svgAttrs : mathAttrs;
  let count = 0;
  const convert = (node: XmlElement | string, depth = 0): ReactNode => {
    if (++count > 12_000 || depth > 80) throw new Error("Drawing too large");
    if (typeof node === "string") return node;
    if (!tags.has(node.name)) return null;
    const properties: Record<string, string | number | boolean> = { key: count };
    for (const [name, value] of Object.entries(node.attributes)) {
      if (kind === "svg" && name === "id" && /^[\w-]+$/.test(value))
        properties.id = `${prefix}-${value}`;
      if (kind === "svg" && /^(marker-start|marker-end)$/.test(name)) {
        const id = /^url\(#([\w-]+)\)$/.exec(value)?.[1];
        if (id) properties[camel[name] ?? name] = `url(#${prefix}-${id})`;
      }
      if (
        attrs.has(name) &&
        !/url\s*\(|[\\<>]|(?:https?|data|javascript):/i.test(value) &&
        !hasControls(value)
      ) {
        properties[camel[name] ?? name] = value;
      }
    }
    if (node === root) {
      if (kind === "math") properties["aria-label"] = label;
      else {
        properties["aria-hidden"] = true;
        properties.focusable = "false";
      }
    }
    return createElement(
      node.name,
      properties,
      node.children.map((child) => convert(child, depth + 1)),
    );
  };
  return convert(root);
}
