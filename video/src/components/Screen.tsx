// An app window made of real screenshots (castor --shot), with a camera that pans and zooms
// onto regions, cross-fades between states, and overlays drawn in screenshot pixels
// (highlights, a cursor, progress fills, a typing reveal).
import React from "react";
import { Easing, Img, interpolate, staticFile, useCurrentFrame } from "remotion";
import { C, EASE_IN_OUT, EASE_OUT, F } from "../theme";

export type Rect = [number, number, number, number]; // x, y, w, h in screenshot pixels
export type CamKey = { f: number; r: Rect };
export type Layer = { src: string; w: number; h: number; at?: number; dur?: number };
export type Box = { x: number; y: number; w: number; h: number };

const ease = Easing.bezier(...EASE_IN_OUT);

export const camAt = (keys: CamKey[], f: number): Rect => {
  if (f <= keys[0].f) return keys[0].r;
  for (let i = 1; i < keys.length; i++) {
    const a = keys[i - 1];
    const b = keys[i];
    if (f <= b.f) {
      const t = ease(Math.min(1, Math.max(0, (f - a.f) / Math.max(1, b.f - a.f))));
      return a.r.map((v, k) => v + (b.r[k] - v) * t) as Rect;
    }
  }
  return keys[keys.length - 1].r;
};

export type View = { z: number; tx: number; ty: number };

export const viewFor = (box: Box, r: Rect, img: { w: number; h: number }, maxZoom: number): View => {
  // never smaller than "cover": the window always fills its frame (no empty margins)
  const cover = Math.max(box.w / img.w, box.h / img.h);
  const z = Math.max(cover, Math.min(box.w / r[2], box.h / r[3], maxZoom));
  const fit = (boxLen: number, imgLen: number, c: number) => {
    const scaled = imgLen * z;
    if (scaled <= boxLen) return (boxLen - scaled) / 2;
    const t = boxLen / 2 - c * z;
    return Math.min(0, Math.max(boxLen - scaled, t));
  };
  return { z, tx: fit(box.w, img.w, r[0] + r[2] / 2), ty: fit(box.h, img.h, r[1] + r[3] / 2) };
};

type Props = {
  box: Box;
  keys: CamKey[];
  layers: Layer[];
  maxZoom?: number;
  /** overlays in screenshot pixels; receives the zoom so strokes can keep their width */
  overlay?: (z: number, frame: number) => React.ReactNode;
  opacity?: number;
  /** window frame (border, radius, shadow); off for floating panels that bring their own */
  frame?: boolean;
  radius?: number;
};

export const Screen: React.FC<Props> = ({ box, keys, layers, maxZoom = 1.8, overlay, opacity = 1, frame = true, radius = 10 }) => {
  const f = useCurrentFrame();
  const base = layers[0];
  const view = viewFor(box, camAt(keys, f), base, maxZoom);
  return (
    <div
      style={{
        position: "absolute",
        left: box.x,
        top: box.y,
        width: box.w,
        height: box.h,
        overflow: "hidden",
        borderRadius: radius,
        border: frame ? `1.5px solid ${C.rule}` : undefined,
        background: C.paper,
        boxShadow: frame ? "0 24px 70px rgba(27, 22, 22, 0.10), 0 2px 6px rgba(27, 22, 22, 0.05)" : undefined,
        opacity,
      }}
    >
      <div
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width: base.w,
          height: base.h,
          transformOrigin: "0 0",
          transform: `translate(${view.tx}px, ${view.ty}px) scale(${view.z})`,
        }}
      >
        {layers.map((l, i) => {
          const o = l.at === undefined ? 1 : interpolate(f, [l.at, l.at + (l.dur ?? 8)], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
          return o > 0 ? (
            <Img
              key={i}
              src={staticFile(l.src)}
              style={{ position: "absolute", left: 0, top: 0, width: l.w, height: l.h, opacity: o, imageRendering: "auto" }}
            />
          ) : null;
        })}
        {overlay ? overlay(view.z, f) : null}
      </div>
    </div>
  );
};

/** Ink outline around a region (in screenshot pixels), drawn in over `dur` frames. */
export const Highlight: React.FC<{ r: Rect; z: number; at: number; f: number; out?: number; wash?: string; pad?: number }> = ({
  r,
  z,
  at,
  f,
  out,
  wash,
  pad = 6,
}) => {
  const p = interpolate(f, [at, at + 10], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(...EASE_OUT) });
  const q = out === undefined ? 1 : interpolate(f, [out, out + 8], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  if (p <= 0 || q <= 0) return null;
  return (
    <div
      style={{
        position: "absolute",
        left: r[0] - pad,
        top: r[1] - pad,
        width: r[2] + pad * 2,
        height: r[3] + pad * 2,
        border: `${2.5 / z}px solid ${C.ink}`,
        borderRadius: 4 / z,
        background: wash,
        opacity: p * q,
        transform: `scale(${1.04 - 0.04 * p})`,
      }}
    />
  );
};

export type CursorKey = { f: number; x: number; y: number };

const cursorAt = (keys: CursorKey[], f: number) => {
  if (f <= keys[0].f) return keys[0];
  for (let i = 1; i < keys.length; i++) {
    const a = keys[i - 1];
    const b = keys[i];
    if (f <= b.f) {
      const t = ease(Math.min(1, Math.max(0, (f - a.f) / Math.max(1, b.f - a.f))));
      return { f, x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
    }
  }
  return keys[keys.length - 1];
};

/** A drawn arrow cursor; at each click frame a small diamond ring opens and fades. */
export const Cursor: React.FC<{ keys: CursorKey[]; clicks?: number[]; z: number; f: number; from?: number; to?: number; carry?: React.ReactNode }> = ({
  keys,
  clicks = [],
  z,
  f,
  from = -Infinity,
  to = Infinity,
  carry,
}) => {
  if (f < from || f > to) return null;
  const p = cursorAt(keys, f);
  const fin = Number.isFinite(from) ? Math.min(1, Math.max(0, (f - from) / 6)) : 1;
  const fout = Number.isFinite(to) ? Math.min(1, Math.max(0, (to - f) / 6)) : 1;
  const fade = Math.min(fin, fout);
  const pressed = clicks.some((c) => f >= c && f < c + 4);
  return (
    <div style={{ position: "absolute", left: p.x, top: p.y, transform: `scale(${1 / z})`, transformOrigin: "0 0", opacity: fade }}>
      {clicks.map((c) => {
        const t = (f - c) / 16;
        if (t < 0 || t > 1) return null;
        const s = 8 + 26 * Easing.bezier(...EASE_OUT)(t);
        return (
          <div
            key={c}
            style={{
              position: "absolute",
              left: -s / 2,
              top: -s / 2,
              width: s,
              height: s,
              border: `2px solid ${C.ink}`,
              transform: "rotate(45deg)",
              opacity: 1 - t,
            }}
          />
        );
      })}
      {carry ? <div style={{ position: "absolute", left: 14, top: 22 }}>{carry}</div> : null}
      <svg width="30" height="36" viewBox="0 0 30 36" style={{ position: "absolute", left: -3, top: -2, transform: pressed ? "scale(0.9)" : undefined, transformOrigin: "3px 2px", filter: "drop-shadow(0 2px 3px rgba(0,0,0,.25))" }}>
        <path d="M3 2 L3 28 L9.5 22 L14 33 L18.5 31 L14 20.5 L23 20.5 Z" fill={C.ink} stroke="#fff" strokeWidth="2" strokeLinejoin="round" />
      </svg>
    </div>
  );
};

/** A file being dragged: page icon + name, like the app's file rows. */
export const FileChip: React.FC<{ name: string; size: string; offset?: number }> = ({ name, size, offset = 0 }) => (
  <div
    style={{
      position: "absolute",
      left: offset * 14,
      top: offset * 12,
      display: "flex",
      alignItems: "center",
      gap: 12,
      padding: "10px 16px 10px 12px",
      background: C.raised,
      border: `1.5px solid ${C.rule}`,
      borderRadius: 4,
      boxShadow: "0 8px 22px rgba(27,22,22,.14)",
      font: `400 17px ${F.sans}`,
      color: C.ink,
      whiteSpace: "nowrap",
    }}
  >
    <svg width="20" height="24" viewBox="0 0 20 24">
      <path d="M2 1 H13 L18 6 V23 H2 Z" fill="#fff" stroke={C.ink} strokeWidth="1.5" />
      <path d="M13 1 V6 H18" fill="none" stroke={C.ink} strokeWidth="1.5" />
      <path d="M5 11 H15 M5 14.5 H15 M5 18 H12" stroke={C.faint} strokeWidth="1.2" />
    </svg>
    <span>{name}</span>
    <span style={{ color: C.muted }}>{size}</span>
  </div>
);
