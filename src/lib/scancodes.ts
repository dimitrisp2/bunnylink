// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
// KeyboardEvent.code -> PC/AT scancode set 1. Extended keys carry the 0xE0 prefix in the high byte.
const E = 0xe000;

export const SCANCODES: Record<string, number> = {
  Escape: 0x01, Digit1: 0x02, Digit2: 0x03, Digit3: 0x04, Digit4: 0x05, Digit5: 0x06, Digit6: 0x07,
  Digit7: 0x08, Digit8: 0x09, Digit9: 0x0a, Digit0: 0x0b, Minus: 0x0c, Equal: 0x0d, Backspace: 0x0e,
  Tab: 0x0f, KeyQ: 0x10, KeyW: 0x11, KeyE: 0x12, KeyR: 0x13, KeyT: 0x14, KeyY: 0x15, KeyU: 0x16,
  KeyI: 0x17, KeyO: 0x18, KeyP: 0x19, BracketLeft: 0x1a, BracketRight: 0x1b, Enter: 0x1c,
  ControlLeft: 0x1d, KeyA: 0x1e, KeyS: 0x1f, KeyD: 0x20, KeyF: 0x21, KeyG: 0x22, KeyH: 0x23,
  KeyJ: 0x24, KeyK: 0x25, KeyL: 0x26, Semicolon: 0x27, Quote: 0x28, Backquote: 0x29, ShiftLeft: 0x2a,
  Backslash: 0x2b, KeyZ: 0x2c, KeyX: 0x2d, KeyC: 0x2e, KeyV: 0x2f, KeyB: 0x30, KeyN: 0x31, KeyM: 0x32,
  Comma: 0x33, Period: 0x34, Slash: 0x35, ShiftRight: 0x36, NumpadMultiply: 0x37, AltLeft: 0x38,
  Space: 0x39, CapsLock: 0x3a, F1: 0x3b, F2: 0x3c, F3: 0x3d, F4: 0x3e, F5: 0x3f, F6: 0x40, F7: 0x41,
  F8: 0x42, F9: 0x43, F10: 0x44, NumLock: 0x45, ScrollLock: 0x46, Numpad7: 0x47, Numpad8: 0x48,
  Numpad9: 0x49, NumpadSubtract: 0x4a, Numpad4: 0x4b, Numpad5: 0x4c, Numpad6: 0x4d, NumpadAdd: 0x4e,
  Numpad1: 0x4f, Numpad2: 0x50, Numpad3: 0x51, Numpad0: 0x52, NumpadDecimal: 0x53, IntlBackslash: 0x56,
  F11: 0x57, F12: 0x58, IntlRo: 0x73, IntlYen: 0x7d,
  NumpadEnter: E | 0x1c, ControlRight: E | 0x1d, NumpadDivide: E | 0x35, PrintScreen: E | 0x37,
  AltRight: E | 0x38, Home: E | 0x47, ArrowUp: E | 0x48, PageUp: E | 0x49, ArrowLeft: E | 0x4b,
  ArrowRight: E | 0x4d, End: E | 0x4f, ArrowDown: E | 0x50, PageDown: E | 0x51, Insert: E | 0x52,
  Delete: E | 0x53, MetaLeft: E | 0x5b, MetaRight: E | 0x5c, ContextMenu: E | 0x5d,
};

export const CTRL_ALT_DEL = [SCANCODES.ControlLeft, SCANCODES.AltLeft, SCANCODES.Delete];

// KeyboardEvent.key -> X11 keysym, for VNC. Printable characters map directly
// (Latin-1 as-is, other Unicode as 0x01000000 + code point).
const KEYSYMS: Record<string, number> = {
  Backspace: 0xff08, Tab: 0xff09, Enter: 0xff0d, Escape: 0xff1b, Delete: 0xffff,
  Home: 0xff50, ArrowLeft: 0xff51, ArrowUp: 0xff52, ArrowRight: 0xff53, ArrowDown: 0xff54,
  PageUp: 0xff55, PageDown: 0xff56, End: 0xff57, Insert: 0xff63, ContextMenu: 0xff67,
  NumLock: 0xff7f, CapsLock: 0xffe5, ScrollLock: 0xff14, Pause: 0xff13, PrintScreen: 0xff61,
  F1: 0xffbe, F2: 0xffbf, F3: 0xffc0, F4: 0xffc1, F5: 0xffc2, F6: 0xffc3,
  F7: 0xffc4, F8: 0xffc5, F9: 0xffc6, F10: 0xffc7, F11: 0xffc8, F12: 0xffc9,
};
const MODIFIERS: Record<string, number> = {
  ShiftLeft: 0xffe1, ShiftRight: 0xffe2, ControlLeft: 0xffe3, ControlRight: 0xffe4,
  AltLeft: 0xffe9, AltRight: 0xfe03, MetaLeft: 0xffeb, MetaRight: 0xffec,
};

export function keysym(e: KeyboardEvent): number | undefined {
  if (MODIFIERS[e.code] !== undefined) return MODIFIERS[e.code];
  if (KEYSYMS[e.key] !== undefined) return KEYSYMS[e.key];
  if ([...e.key].length === 1) {
    const cp = e.key.codePointAt(0)!;
    return cp < 0x100 ? cp : 0x01000000 + cp;
  }
  return undefined;
}

export const CTRL_ALT_DEL_KEYSYMS = [0xffe3, 0xffe9, 0xffff];
