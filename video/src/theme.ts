// Tokens from docs/DESIGN.md (light) and the family pastels of the TeXtured badges.
import { loadFont } from "@remotion/fonts";
import { staticFile } from "remotion";

export const C = {
  paper: "#fcfbf9",
  surface: "#f4f2ee",
  raised: "#ffffff",
  ink: "#1b1616",
  muted: "#5c5a55",
  faint: "#8c8a84",
  rule: "#d9d6cf",
  good: "#2e7d4f",
  link: "#2f6db5",
  ghost: "rgba(27, 22, 22, 0.09)",
};

export type Family = "pdf" | "image" | "video" | "audio" | "create" | "files";
export const FAMILIES: Family[] = ["pdf", "image", "video", "audio", "create", "files"];
// tint = card wash, stroke = diamond swatch (as in video/social/src/card.html)
export const FAM: Record<Family, { tint: string; stroke: string }> = {
  pdf: { tint: "#fbe2ff", stroke: "#c43fd0" },
  image: { tint: "#ccfaff", stroke: "#16a7b8" },
  video: { tint: "#ade1ff", stroke: "#2f8fd8" },
  audio: { tint: "#aafaba", stroke: "#2fa652" },
  create: { tint: "#ffeca4", stroke: "#c39a12" },
  files: { tint: "#ffdcc8", stroke: "#dc7340" },
};

export const F = {
  roman: "C Roman, serif",
  slant: "C Slant, serif",
  sans: "C Sans, sans-serif",
  mono: "C Mono, monospace",
  chorus: "C Chorus, cursive",
};

let loaded: Promise<unknown> | null = null;
export const loadFonts = () => {
  if (!loaded) {
    const f = (family: string, file: string, weight = "400", style = "normal") =>
      loadFont({ family, url: staticFile(`fonts/${file}`), weight, style, format: "opentype" });
    loaded = Promise.all([
      f("C Roman", "lmroman10-regular.otf"),
      f("C Roman", "lmroman10-bold.otf", "700"),
      f("C Slant", "lmromanslant10-regular.otf"),
      f("C Sans", "lmsans10-regular.otf"),
      f("C Sans", "lmsans10-bold.otf", "700"),
      f("C Mono", "lmmono10-regular.otf"),
      f("C Chorus", "texgyrechorus-mediumitalic.otf", "400", "italic"),
    ]);
  }
  return loaded;
};

// Calm easing: ease-out for arrivals, ease-in-out for camera moves. Nothing bounces.
export const EASE_OUT = [0.16, 1, 0.3, 1] as const;
export const EASE_IN_OUT = [0.65, 0, 0.35, 1] as const;
