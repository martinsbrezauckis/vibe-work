/** Math delimiters for react-markdown's parser. Keeps escapes/code intact and refuses currency-like dollar pairs. */
import type { Tokenizer, State, Extension as SyntaxExtension } from "micromark-util-types";
import type { Extension } from "mdast-util-from-markdown";
import type { Processor } from "unified";

declare module "unified" {
  interface Data {
    micromarkExtensions?: SyntaxExtension[];
    fromMarkdownExtensions?: Extension[];
  }
}

declare module "micromark-util-types" {
  interface TokenTypeMap {
    chatMath: "chatMath";
    chatMathBreak: "chatMathBreak";
    chatMathData: "chatMathData";
  }
}

const whitespace = (code: number | null) => code !== null && (code <= 32 || code === 127);
const digit = (code: number | null) => code !== null && code >= 48 && code <= 57;

const tokenize: Tokenizer = function (effects, ok, nok) {
  let dollar = false;
  let display = false;
  let closer = 0;
  let previous: number | null = null;
  let count = 0;
  const start: State = (code) => {
    dollar = code === 36;
    effects.enter("chatMath");
    effects.enter("chatMathData");
    effects.consume(code);
    return opening;
  };
  const opening: State = (code) => {
    if (dollar) {
      if (code === 36) {
        display = true;
        effects.consume(code);
        return body;
      }
      if (whitespace(code)) return nok(code);
      return body(code);
    }
    if (code !== 40 && code !== 91) return nok(code);
    display = code === 91;
    closer = display ? 93 : 41;
    effects.consume(code);
    return body;
  };
  const body: State = (code) => {
    if (code === null || count > 8000 || (!display && code < 0)) return nok(code);
    if (dollar && code === 36) {
      if (!display && (count === 0 || whitespace(previous))) return nok(code);
      effects.consume(code);
      return display ? secondDollar : end;
    }
    if (code === 92) {
      effects.consume(code);
      return escaped;
    }
    count++;
    previous = code;
    if (code <= -3) {
      effects.exit("chatMathData");
      effects.enter("chatMathBreak");
      effects.consume(code);
      effects.exit("chatMathBreak");
      return (next) => {
        effects.enter("chatMathData");
        return body(next);
      };
    }
    effects.consume(code);
    return body;
  };
  const escaped: State = (code) => {
    if (code === null) return nok(code);
    if (code <= -3) return body(code);
    if (!dollar && code === closer) {
      effects.consume(code);
      return end;
    }
    count += 2;
    previous = code;
    effects.consume(code);
    return body;
  };
  const secondDollar: State = (code) => {
    if (code !== 36) return nok(code);
    effects.consume(code);
    return end;
  };
  const end: State = (code) => {
    // "$5 and $10" has no closing delimiter: the second dollar introduces another price.
    if (count === 0 || (dollar && !display && digit(code))) return nok(code);
    effects.exit("chatMathData");
    effects.exit("chatMath");
    return ok(code);
  };
  return start;
};

const fromMarkdown: Extension = {
  enter: {
    chatMath(token) {
      const raw = this.sliceSerialize(token);
      const display = raw.startsWith("$$") || raw.startsWith("\\[");
      const width = raw.startsWith("$") && !display ? 1 : 2;
      this.enter(
        {
          type: "inlineCode",
          value: raw.slice(width, -width).trim(),
          data: {
            hName: "span",
            hProperties: {
              "data-math": raw.slice(width, -width).trim(),
              "data-display": display ? "display" : "inline",
              "data-source": raw,
            },
          },
        },
        token,
      );
    },
  },
  exit: {
    chatMath(token) {
      this.exit(token);
    },
  },
};

/** Adds syntax at parsing time, so TeX underscores, escapes, links and code cannot be mistaken for Markdown. */
export function remarkMath(this: Processor): void {
  const data = this.data();
  const syntax = { text: { 36: { tokenize }, 92: { tokenize } } };
  (data.micromarkExtensions ??= []).push(syntax);
  (data.fromMarkdownExtensions ??= []).push(fromMarkdown);
}
