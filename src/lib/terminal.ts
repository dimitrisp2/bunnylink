// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
import type { ITerminalOptions, ITheme } from "@xterm/xterm";
import type { Settings } from "./api";

/** A terminal colour scheme. `ansi` holds the 8 normal colours, then the 8 bright ones. */
export interface TermScheme {
  id: string;
  name: string;
  background: string;
  foreground: string;
  cursor: string;
  /** Selection highlight; when missing, the text colour at a third opacity. */
  selection?: string;
  ansi: string[];
}

export const APP_SCHEME = "app";

// ANSI palettes for the app's own scheme, tuned for contrast on each background.
const APP_ANSI = {
  dark: ["#1b232b", "#ef7a6c", "#7fd39b", "#e8c06a", "#6fa8f0", "#c79bf0", "#5fd0d2", "#d3dde6", "#5c6b79", "#ff9a8c", "#9be8b4", "#f5d58c", "#93c0ff", "#dcb8ff", "#86e6e7", "#f4f7fa"],
  light: ["#1d2833", "#b8322a", "#1d7a44", "#8a5d00", "#1f5fbf", "#8a3fb3", "#0d7f86", "#c5ccd3", "#5b6876", "#d4483e", "#25924f", "#a87200", "#2f72d6", "#a052cc", "#11959d", "#eef1f4"],
};

const SOLARIZED = ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5", "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3"];
const TANGO = ["#000000", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf", "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec"];

export const BUILTIN_SCHEMES: TermScheme[] = [
  {
    id: "campbell", name: "Campbell", background: "#0c0c0c", foreground: "#cccccc", cursor: "#ffffff",
    ansi: ["#0c0c0c", "#c50f1f", "#13a10e", "#c19c00", "#0037da", "#881798", "#3a96dd", "#cccccc", "#767676", "#e74856", "#16c60c", "#f9f1a5", "#3b78ff", "#b4009e", "#61d6d6", "#f2f2f2"],
  },
  {
    id: "dracula", name: "Dracula", background: "#282a36", foreground: "#f8f8f2", cursor: "#f8f8f2", selection: "#44475a",
    ansi: ["#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2", "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff"],
  },
  {
    id: "gruvbox-dark", name: "Gruvbox Dark", background: "#282828", foreground: "#ebdbb2", cursor: "#ebdbb2", selection: "#504945",
    ansi: ["#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984", "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2"],
  },
  {
    id: "nord", name: "Nord", background: "#2e3440", foreground: "#d8dee9", cursor: "#d8dee9", selection: "#434c5e",
    ansi: ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0", "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"],
  },
  {
    id: "one-dark", name: "One Dark", background: "#282c34", foreground: "#dcdfe4", cursor: "#a3b3cc", selection: "#474e5d",
    ansi: ["#282c34", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#dcdfe4", "#5a6374", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#dcdfe4"],
  },
  { id: "solarized-dark", name: "Solarized Dark", background: "#002b36", foreground: "#839496", cursor: "#93a1a1", selection: "#073642", ansi: SOLARIZED },
  { id: "solarized-light", name: "Solarized Light", background: "#fdf6e3", foreground: "#657b83", cursor: "#586e75", selection: "#eee8d5", ansi: SOLARIZED },
  { id: "tango-dark", name: "Tango Dark", background: "#000000", foreground: "#d3d7cf", cursor: "#d3d7cf", ansi: TANGO },
  { id: "tango-light", name: "Tango Light", background: "#ffffff", foreground: "#555753", cursor: "#555753", ansi: TANGO },
];

/** The app theme's own scheme, read from the current CSS colours. */
function appScheme(mode: "dark" | "light"): TermScheme {
  const css = getComputedStyle(document.documentElement);
  const v = (n: string) => css.getPropertyValue(n).trim();
  return {
    id: APP_SCHEME, name: "Match app theme",
    background: v("--term-bg"), foreground: v("--term-fg"), cursor: v("--accent"), selection: v("--sel"),
    ansi: APP_ANSI[mode],
  };
}

export const schemeById = (id: string, mode: "dark" | "light") =>
  BUILTIN_SCHEMES.find((s) => s.id === id) ?? appScheme(mode);

const ANSI_KEYS = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

export function xtermTheme(s: TermScheme): ITheme {
  const theme: ITheme = {
    background: s.background,
    foreground: s.foreground,
    cursor: s.cursor,
    cursorAccent: s.background,
    selectionBackground: s.selection ?? `${s.foreground}55`,
  };
  ANSI_KEYS.forEach((k, i) => {
    theme[k] = s.ansi[i];
    theme[`bright${k[0].toUpperCase()}${k.slice(1)}` as keyof ITheme] = s.ansi[i + 8] as never;
  });
  return theme;
}

/** Whether a #rrggbb colour is light, so highlights can be picked to show on it. */
export function isLight(hex: string) {
  const n = parseInt(hex.replace("#", "").slice(0, 6), 16);
  if (Number.isNaN(n)) return false;
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  return 0.299 * r + 0.587 * g + 0.114 * b > 140;
}

const monoFonts = () => getComputedStyle(document.documentElement).getPropertyValue("--mono").trim();

export function fontStack(family: string | null) {
  const name = family?.trim().replace(/["\\]/g, "");
  return name ? `"${name}", ${monoFonts()}` : monoFonts();
}

/** How terminals look under the given settings, as xterm options plus the scheme used. */
export function terminalLook(s: Settings, mode: "dark" | "light") {
  const scheme = schemeById(s.terminalScheme, mode);
  const options: ITerminalOptions = {
    fontFamily: fontStack(s.terminalFontFamily),
    fontSize: s.terminalFontSize,
    lineHeight: s.terminalLineHeight,
    letterSpacing: s.terminalLetterSpacing,
    cursorStyle: s.terminalCursor,
    theme: xtermTheme(scheme),
  };
  return { options, scheme };
}

/** Whether a font is installed: text in it measures differently from the generic fallbacks. */
export function fontInstalled(name: string) {
  const ctx = document.createElement("canvas").getContext("2d");
  if (!ctx) return true;
  const sample = "mmmmmmmmmmlli1WQ@";
  return ["monospace", "serif", "sans-serif"].some((generic) => {
    ctx.font = `72px ${generic}`;
    const base = ctx.measureText(sample).width;
    ctx.font = `72px "${name.replace(/["\\]/g, "")}", ${generic}`;
    return ctx.measureText(sample).width !== base;
  });
}

/** Monospace fonts offered in Settings; any installed font can also be typed in. */
export const SUGGESTED_FONTS = [
  "Cascadia Mono", "Cascadia Code", "JetBrains Mono", "Fira Code", "Consolas", "Hack", "Source Code Pro", "Ubuntu Mono",
  "DejaVu Sans Mono", "Iosevka", "MesloLGS NF", "CaskaydiaCove Nerd Font", "JetBrainsMono Nerd Font", "Hack Nerd Font",
];

let audio: AudioContext | undefined;
let lastBell = 0;

/** A short, soft two-tone blip. Bells closer than 100 ms apart play once. */
export function playBell() {
  const now = performance.now();
  if (now - lastBell < 100) return;
  lastBell = now;
  audio ??= new AudioContext();
  const t = audio.currentTime;
  const gain = audio.createGain();
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(0.12, t + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.15);
  gain.connect(audio.destination);
  const osc = audio.createOscillator();
  osc.type = "sine";
  osc.frequency.setValueAtTime(880, t);
  osc.frequency.setValueAtTime(1320, t + 0.05);
  osc.connect(gain);
  osc.start(t);
  osc.stop(t + 0.16);
}
