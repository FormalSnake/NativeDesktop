// Pure per-cell edit resolution for OtpInput. `<textinput>` only reports
// `onChanged`/`text` (no per-key event on the widget yet), so a paste into
// one box arrives as that box's `text` suddenly holding every pasted
// character, which is what tells a paste apart from a normal keystroke.
//
// `activeIndex` is the cell focus moves to next. OtpInput sends the `focus`
// command to that cell, which is what makes typing, backspace and paste
// advance the caret without the user clicking between boxes.

export interface OtpEdit {
  value: string;
  activeIndex: number;
}

export function otpChars(value: string, length: number): string[] {
  const chars = new Array<string>(length).fill("");
  for (let i = 0; i < Math.min(value.length, length); i++) chars[i] = value[i]!;
  return chars;
}

/** One cell's onChanged fired with `text`. */
export function otpCellChanged(value: string, length: number, index: number, text: string): OtpEdit {
  const chars = otpChars(value, length);

  if (text.length > 1) {
    // A paste landed in this box: distribute it across this cell and the
    // ones that follow.
    let i = index;
    for (const ch of text) {
      if (i >= length) break;
      chars[i] = ch;
      i++;
    }
    return { value: chars.join(""), activeIndex: Math.min(i, length - 1) };
  }

  if (text.length === 0) {
    // Backspace on a filled cell just clears it, staying put. Backspace on
    // an already-empty cell would step back one, but that keystroke never
    // changes this cell's text, so it never reaches onChanged; only a real
    // key event could report it.
    const wasFilled = chars[index] !== "";
    chars[index] = "";
    return { value: chars.join(""), activeIndex: wasFilled ? index : Math.max(index - 1, 0) };
  }

  chars[index] = text[text.length - 1]!;
  return { value: chars.join(""), activeIndex: Math.min(index + 1, length - 1) };
}
