// Playwright-shaped selector strings, parsed into the SelectorPart list the
// matcher and Locator work on. A selector is one or more parts joined by
// ">>"; each engine part descends into the previous part's subtree, while the
// positional and filter parts refine the current match set in place.
//
//   testid=save-btn
//   role=button[name="Save"][exact] >> nth=1
//   type=Table >> has-text=Ada >> first
//
// Every part is representable as a string, so a Locator built through the
// getBy*/filter helpers can print itself back as a selector in an error.

export interface TextSpec {
  /** Substring or exact literal; absent when `regex` is set. */
  value?: string;
  regex?: RegExp;
  /** Equality after whitespace normalisation, rather than a case-insensitive substring. */
  exact?: boolean;
}

export type SelectorPart =
  | { kind: "testid"; value: string }
  | ({ kind: "role"; role: string; checked?: boolean; disabled?: boolean } & { name?: TextSpec })
  | ({ kind: "text" | "label" | "placeholder" } & TextSpec)
  | { kind: "type"; value: string }
  | { kind: "nth"; index: number }
  | ({ kind: "has-text" } & TextSpec)
  | { kind: "has" | "has-not" | "and"; parts: SelectorPart[] };

const TEXT_ENGINES = new Set(["text", "label", "placeholder"]);

/** Splits on top-level ">>", ignoring separators inside quotes, brackets,
 * parens and regex literals. */
function splitParts(selector: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let quote: string | null = null;
  let inRegex = false;
  let start = 0;
  for (let i = 0; i < selector.length; i++) {
    const c = selector[i]!;
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
      continue;
    }
    if (inRegex) {
      if (c === "\\") i++;
      else if (c === "/") inRegex = false;
      continue;
    }
    if (c === '"' || c === "'") quote = c;
    else if (c === "/" && selector[i - 1] === "=") inRegex = true;
    else if (c === "[" || c === "(") depth++;
    else if (c === "]" || c === ")") depth--;
    else if (depth === 0 && c === ">" && selector[i + 1] === ">") {
      out.push(selector.slice(start, i));
      i++;
      start = i + 1;
    }
  }
  out.push(selector.slice(start));
  return out.map((p) => p.trim()).filter((p) => p.length > 0);
}

function parseRegex(body: string): RegExp | null {
  if (!body.startsWith("/")) return null;
  const end = body.lastIndexOf("/");
  if (end <= 0) return null;
  return new RegExp(body.slice(1, end), body.slice(end + 1));
}

function parseTextSpec(body: string): TextSpec {
  const trimmed = body.trim();
  const regex = parseRegex(trimmed);
  if (regex) return { regex };
  if (trimmed.length >= 2 && (trimmed[0] === '"' || trimmed[0] === "'") && trimmed.at(-1) === trimmed[0]) {
    return { value: unquote(trimmed), exact: true };
  }
  return { value: trimmed };
}

function unquote(s: string): string {
  const q = s[0]!;
  let out = "";
  for (let i = 1; i < s.length - 1; i++) {
    const c = s[i]!;
    if (c === "\\") {
      const next = s[++i];
      out += next === "n" ? "\n" : next === "t" ? "\t" : (next ?? "");
    } else if (c !== q) {
      out += c;
    }
  }
  return out;
}

/** `[name="Save"]`, `[name=/sa/i]`, `[exact]`, `[checked]`, `[disabled]`. */
function parseRoleAttrs(part: SelectorPart & { kind: "role" }, attrs: string, selector: string): void {
  const re = /\[([a-zA-Z-]+)(?:=([^\]]*))?\]/g;
  let m: RegExpExecArray | null;
  let consumed = 0;
  while ((m = re.exec(attrs))) {
    consumed = re.lastIndex;
    const key = m[1]!;
    const raw = m[2];
    switch (key) {
      case "name":
        part.name = parseTextSpec(raw ?? "");
        break;
      case "exact":
        part.name = { ...(part.name ?? {}), exact: raw !== "false" };
        break;
      case "checked":
        part.checked = raw !== "false";
        break;
      case "disabled":
        part.disabled = raw !== "false";
        break;
      default:
        throw new Error(`selector "${selector}": unknown role attribute [${key}]`);
    }
  }
  if (consumed !== attrs.length) throw new Error(`selector "${selector}": malformed role attributes "${attrs}"`);
}

function parseNested(body: string, selector: string): SelectorPart[] {
  const trimmed = body.trim();
  const inner = trimmed.startsWith("(") && trimmed.endsWith(")") ? trimmed.slice(1, -1) : trimmed;
  const parts = parseSelector(inner);
  if (!parts.length) throw new Error(`selector "${selector}": empty nested selector`);
  return parts;
}

export function parseSelector(selector: string): SelectorPart[] {
  const out: SelectorPart[] = [];
  for (const raw of splitParts(selector)) {
    if (raw === "first") {
      out.push({ kind: "nth", index: 0 });
      continue;
    }
    if (raw === "last") {
      out.push({ kind: "nth", index: -1 });
      continue;
    }
    const eq = raw.indexOf("=");
    if (eq < 0) throw new Error(`selector "${selector}": part "${raw}" has no engine (want engine=value)`);
    const engine = raw.slice(0, eq).trim().toLowerCase();
    const body = raw.slice(eq + 1).trim();
    if (engine === "nth") {
      const index = Number(body);
      if (!Number.isInteger(index)) throw new Error(`selector "${selector}": nth=${body} is not an integer`);
      out.push({ kind: "nth", index });
    } else if (engine === "testid" || engine === "test-id") {
      out.push({ kind: "testid", value: body.startsWith('"') || body.startsWith("'") ? unquote(body) : body });
    } else if (engine === "type") {
      out.push({ kind: "type", value: body });
    } else if (engine === "role") {
      const bracket = body.indexOf("[");
      const part: SelectorPart & { kind: "role" } = {
        kind: "role",
        role: (bracket < 0 ? body : body.slice(0, bracket)).trim(),
      };
      if (bracket >= 0) parseRoleAttrs(part, body.slice(bracket), selector);
      if (!part.role) throw new Error(`selector "${selector}": role= needs a role name`);
      out.push(part);
    } else if (TEXT_ENGINES.has(engine)) {
      out.push({ kind: engine as "text" | "label" | "placeholder", ...parseTextSpec(body) });
    } else if (engine === "has-text") {
      out.push({ kind: "has-text", ...parseTextSpec(body) });
    } else if (engine === "has" || engine === "has-not" || engine === "and") {
      out.push({ kind: engine, parts: parseNested(body, selector) });
    } else {
      throw new Error(`selector "${selector}": unknown engine "${engine}"`);
    }
  }
  return out;
}

function formatTextSpec(spec: TextSpec): string {
  if (spec.regex) return String(spec.regex);
  if (spec.exact) return JSON.stringify(spec.value ?? "");
  return spec.value ?? "";
}

function formatPart(part: SelectorPart): string {
  switch (part.kind) {
    case "testid":
      return `testid=${part.value}`;
    case "type":
      return `type=${part.value}`;
    case "nth":
      return `nth=${part.index}`;
    case "role": {
      let out = `role=${part.role}`;
      // A quoted name already means exact, so [exact] is only emitted for the
      // degenerate [exact]-with-no-name form.
      if (part.name?.regex || part.name?.value) out += `[name=${formatTextSpec(part.name)}]`;
      else if (part.name?.exact) out += "[exact]";
      if (part.checked !== undefined) out += part.checked ? "[checked]" : "[checked=false]";
      if (part.disabled !== undefined) out += part.disabled ? "[disabled]" : "[disabled=false]";
      return out;
    }
    case "has":
    case "has-not":
    case "and":
      return `${part.kind}=(${formatSelector(part.parts)})`;
    default:
      return `${part.kind}=${formatTextSpec(part)}`;
  }
}

export function formatSelector(parts: SelectorPart[]): string {
  return parts.map(formatPart).join(" >> ");
}

/** The literal a candidate list ranks against: the name/id the last engine
 * part was reaching for. */
export function intendedName(parts: SelectorPart[]): string | undefined {
  for (let i = parts.length - 1; i >= 0; i--) {
    const part = parts[i]!;
    if (part.kind === "testid" || part.kind === "type") return part.value;
    if (part.kind === "role") return part.name?.value ?? part.role;
    if (part.kind === "text" || part.kind === "label" || part.kind === "placeholder" || part.kind === "has-text") {
      return part.value ?? part.regex?.source;
    }
  }
  return undefined;
}
