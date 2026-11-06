import React from "react";
import { Easing, Html5Audio, interpolate, Sequence, staticFile, useCurrentFrame } from "remotion";
import { EASE_OUT } from "../theme";

export const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;
export const easeOut = Easing.bezier(...EASE_OUT);

/** 0 → 1 between frames a and b (ease-out). */
export const io = (f: number, a: number, b: number) => interpolate(f, [a, b], [0, 1], { ...clamp, easing: easeOut });

/** Scene opacity: fades in over the first `fin` frames and out over the last `fout`. */
export const useSceneFade = (dur: number, fin = 12, fout = 12) => {
  const f = useCurrentFrame();
  return Math.min(interpolate(f, [0, fin], [0, 1], clamp), interpolate(f, [dur - fout, dur], [1, 0], clamp));
};

/** Text that rises 14 px and fades in from frame `at`. */
export const Rise: React.FC<{ at: number; children: React.ReactNode; dy?: number; style?: React.CSSProperties; dur?: number }> = ({
  at,
  children,
  dy = 14,
  style,
  dur = 18,
}) => {
  const f = useCurrentFrame();
  const p = io(f, at, at + dur);
  return <div style={{ opacity: p, transform: `translateY(${(1 - p) * dy}px)`, ...style }}>{children}</div>;
};

/** A UI sound at a frame (relative to the enclosing sequence). */
export const Sfx: React.FC<{ at: number; src: string; vol?: number }> = ({ at, src, vol = 1 }) => (
  <Sequence from={Math.round(at)} durationInFrames={60} layout="none">
    <Html5Audio src={staticFile(`sfx/${src}`)} volume={vol} />
  </Sequence>
);

/** Key taps for typed text: one tap every `every` frames. */
export const Typing: React.FC<{ from: number; count: number; every: number; vol?: number }> = ({ from, count, every, vol = 0.6 }) => (
  <>
    {Array.from({ length: count }, (_, i) => (
      <Sfx key={i} at={from + i * every} src={`key${(i * 7) % 4}.wav`} vol={vol} />
    ))}
  </>
);

/** Characters of `text` visible at frame f when typing starts at `from`, one every `every` frames. */
export const typed = (text: string, f: number, from: number, every: number) => {
  const n = Math.max(0, Math.min(text.length, Math.floor((f - from) / every) + 1));
  return f < from ? "" : [...text].slice(0, n).join("");
};
