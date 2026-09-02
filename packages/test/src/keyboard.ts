// Playwright's Keyboard and Mouse over the `keys`, `pointer` and `drag` RPCs.
//
// The host chord spec takes lowercase modifier tokens (cmd/ctrl/shift/alt)
// plus one key token, and lowercases every part it parses, so an uppercase
// letter has to arrive as an explicit shift. Names outside the host's table
// (Home/End/PageUp/PageDown/F1-F12) are passed through lowercased rather than
// rejected here, so the client does not have to ship a new release when the
// host's table grows.
import type { LocatorClient } from "./locator.ts";

const MODIFIERS: Record<string, string> = {
  meta: "cmd",
  cmd: "cmd",
  command: "cmd",
  control: "ctrl",
  ctrl: "ctrl",
  shift: "shift",
  alt: "alt",
  option: "alt",
};

const KEY_NAMES: Record<string, string> = {
  arrowleft: "left",
  arrowright: "right",
  arrowup: "up",
  arrowdown: "down",
  esc: "escape",
  escape: "escape",
  enter: "enter",
  return: "enter",
  tab: "tab",
  space: "space",
  " ": "space",
  backspace: "backspace",
  delete: "delete",
};

/** "Meta+Shift+A" -> "cmd+shift+a"; "ArrowDown" -> "down"; "a" -> "a". */
export function toChord(key: string): string {
  const parts = key.split("+").filter((p) => p.length > 0);
  const mods: string[] = [];
  let token = "";
  for (const raw of parts) {
    const mod = MODIFIERS[raw.toLowerCase()];
    if (mod) {
      if (!mods.includes(mod)) mods.push(mod);
      continue;
    }
    token = raw;
  }
  if (!token) throw new Error(`press(${JSON.stringify(key)}): no key left after the modifiers`);
  if (token.length === 1 && token !== token.toLowerCase()) {
    if (!mods.includes("shift")) mods.push("shift");
    token = token.toLowerCase();
  }
  const named = KEY_NAMES[token.toLowerCase()];
  const resolved = named ?? (token.length === 1 ? token : token.toLowerCase());
  return mods.length ? [...mods, resolved].join("+") : resolved;
}

export class Keyboard {
  constructor(
    private readonly client: LocatorClient,
    private readonly window?: number,
  ) {}

  /** One chord, in Playwright key names: press("Meta+A"), press("Escape"). */
  async press(key: string): Promise<void> {
    await this.client.callRpc("keys", { keys: toChord(key), window: this.window });
  }

  /** Types the literal text into the focused widget, one keystroke per
   * character. A single named key would be read as a chord by the host, so
   * route those through press(). */
  async type(text: string): Promise<void> {
    if (!text) return;
    await this.client.callRpc("keys", { keys: text, window: this.window });
  }

  insertText(text: string): Promise<void> {
    return this.type(text);
  }
}

export interface MouseButtonOptions {
  button?: "left" | "right";
  clickCount?: number;
}

export class Mouse {
  private x = 0;
  private y = 0;

  constructor(
    private readonly client: LocatorClient,
    private readonly window?: number,
  ) {}

  async move(x: number, y: number): Promise<void> {
    this.x = x;
    this.y = y;
    await this.client.callRpc("pointer", { phase: "move", x, y, window: this.window });
  }

  async down(opts: MouseButtonOptions = {}): Promise<void> {
    await this.client.callRpc("pointer", { phase: "down", x: this.x, y: this.y, ...opts, window: this.window });
  }

  async up(opts: MouseButtonOptions = {}): Promise<void> {
    await this.client.callRpc("pointer", { phase: "up", x: this.x, y: this.y, ...opts, window: this.window });
  }

  async click(x: number, y: number, opts: MouseButtonOptions = {}): Promise<void> {
    this.x = x;
    this.y = y;
    await this.down(opts);
    await this.up(opts);
  }

  async dblclick(x: number, y: number, opts: MouseButtonOptions = {}): Promise<void> {
    await this.click(x, y, opts);
    await this.click(x, y, { ...opts, clickCount: 2 });
  }

  /** One posted down/dragged/up batch, so a native tracking loop consumes it
   * as a real gesture. */
  async dragTo(
    from: { x: number; y: number },
    to: { x: number; y: number },
    opts: { steps?: number; durationMs?: number; button?: "left" | "right" } = {},
  ): Promise<void> {
    this.x = to.x;
    this.y = to.y;
    await this.client.callRpc("drag", {
      fromX: from.x,
      fromY: from.y,
      toX: to.x,
      toY: to.y,
      ...opts,
      window: this.window,
    });
  }
}
