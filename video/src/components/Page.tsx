// Paper, running head (label · number above a rule ending in a diamond) and the
// "Figure n / …" caption line: the furniture of every page of the video.
import React from "react";
import { AbsoluteFill, useVideoConfig } from "remotion";
import { C, F } from "../theme";

export const useLayout = () => {
  const { width, height } = useVideoConfig();
  const v = height > width;
  return { W: width, H: height, v, m: v ? 72 : 96 };
};

// Static paper grain: a fixed turbulence at very low opacity (kept still so it costs no bits).
export const Paper: React.FC<{ children?: React.ReactNode }> = ({ children }) => (
  <AbsoluteFill style={{ background: C.paper }}>
    <svg width="100%" height="100%" style={{ position: "absolute", inset: 0, opacity: 0.05 }}>
      <filter id="grain">
        <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="4" stitchTiles="stitch" />
        <feColorMatrix type="saturate" values="0" />
      </filter>
      <rect width="100%" height="100%" filter="url(#grain)" />
    </svg>
    {children}
  </AbsoluteFill>
);

export const Diamond: React.FC<{ size: number; color?: string; fill?: string; stroke?: number; style?: React.CSSProperties }> = ({
  size,
  color = C.ink,
  fill = "transparent",
  stroke = 1.5,
  style,
}) => (
  <span
    style={{
      display: "inline-block",
      width: size,
      height: size,
      border: `${stroke}px solid ${color}`,
      background: fill,
      transform: "rotate(45deg)",
      flex: "none",
      ...style,
    }}
  />
);

/** Running head: label on the left, number on the right above a rule that ends in a diamond. */
export const RunningHead: React.FC<{ label: string; num: string; opacity?: number }> = ({ label, num, opacity = 1 }) => {
  const { m, v } = useLayout();
  const fs = v ? 26 : 26;
  return (
    <div
      style={{
        position: "absolute",
        left: m,
        right: m,
        top: v ? 64 : 44,
        height: 48,
        opacity,
        display: "flex",
        justifyContent: "space-between",
        alignItems: "baseline",
        font: `400 ${fs}px ${F.sans}`,
        color: C.muted,
      }}
    >
      <span>{label}</span>
      <b style={{ color: C.ink, fontSize: fs + 2, fontFamily: F.sans }}>{num}</b>
      <span
        style={{
          position: "absolute",
          left: "50%",
          right: 0,
          bottom: 0,
          height: 1.5,
          background: `linear-gradient(to right, transparent, ${C.faint})`,
        }}
      />
      <span
        style={{
          position: "absolute",
          right: -10,
          bottom: -4.5,
          width: 9,
          height: 9,
          background: C.paper,
          border: `1.5px solid ${C.faint}`,
          transform: "rotate(45deg)",
        }}
      />
    </div>
  );
};

/** "Figure n / text" at the bottom left, the address at the bottom right. */
export const Caption: React.FC<{ label: string; n: number | string; text: string; url?: string; opacity?: number }> = ({
  label,
  n,
  text,
  url,
  opacity = 1,
}) => {
  const { m, v } = useLayout();
  return (
    <div
      style={{
        position: "absolute",
        left: m,
        right: m,
        bottom: v ? 120 : 44,
        display: "flex",
        flexDirection: v ? "column" : "row",
        justifyContent: "space-between",
        alignItems: v ? "flex-start" : "baseline",
        gap: v ? 18 : 40,
        font: `400 ${v ? 32 : 28}px/1.35 ${F.roman}`,
        color: C.muted,
        opacity,
      }}
    >
      <span>
        <b style={{ fontFamily: F.sans, fontWeight: 700, color: C.ink, marginRight: "0.25em" }}>
          {label} {n} /
        </b>{" "}
        {text}
      </span>
      {url && !v ? <span style={{ fontFamily: F.sans, whiteSpace: "nowrap" }}>{url}</span> : null}
    </div>
  );
};

/** Section title: ghost Chorus initial behind "n.  Title" (like the TeXtured headings). */
export const SectionTitle: React.FC<{ num?: string; title: string; size: number; ghost?: string; style?: React.CSSProperties }> = ({
  num,
  title,
  size,
  ghost,
  style,
}) => (
  <div style={{ position: "relative", fontSize: size, ...style }}>
    {ghost ? (
      <span
        style={{
          position: "absolute",
          left: "-0.04em",
          top: "-0.55em",
          font: `italic 400 2.05em/1 ${F.chorus}`,
          color: C.ghost,
        }}
      >
        {ghost}
      </span>
    ) : null}
    <div style={{ position: "relative", fontFamily: F.sans, fontWeight: 700, lineHeight: 1.08, color: C.ink }}>
      {num ? <span style={{ marginRight: "0.45em" }}>{num}</span> : null}
      {title}
    </div>
  </div>
);
