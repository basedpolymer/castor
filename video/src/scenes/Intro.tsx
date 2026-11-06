// 1. Paper; the bitten diamond draws itself (TikZ-like stroke), is inked, bitten; "Castor".
import React from "react";
import { AbsoluteFill, useCurrentFrame } from "remotion";
import { Mark, prog } from "../components/Mark";
import { useLayout } from "../components/Page";
import { C, F } from "../theme";
import { io, Sfx, useSceneFade } from "./common";

export const Intro: React.FC<{ dur: number }> = ({ dur }) => {
  const f = useCurrentFrame();
  const { v } = useLayout();
  const fade = useSceneFade(dur, 1, 14);
  const size = v ? 380 : 320;
  const title = io(f, 104, 130);
  return (
    <AbsoluteFill style={{ opacity: fade, alignItems: "center", justifyContent: "center" }}>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: v ? 70 : 46, transform: `translateY(${v ? -60 : -10}px)` }}>
        <Mark
          id="intro"
          size={size}
          draw={prog(f, 8, 58)}
          fill={prog(f, 56, 76)}
          bite={prog(f, 74, 92)}
          crumbs={prog(f, 88, 96)}
          letter={prog(f, 92, 112)}
        />
        <div style={{ position: "relative", opacity: title, transform: `translateY(${(1 - title) * 12}px)` }}>
          <span
            style={{
              position: "absolute",
              left: "-0.06em",
              top: "-0.62em",
              font: `italic 400 ${v ? 250 : 220}px/1 ${F.chorus}`,
              color: C.ghost,
            }}
          >
            C
          </span>
          <div style={{ position: "relative", font: `700 ${v ? 132 : 120}px/1 ${F.sans}`, color: C.ink, letterSpacing: "-0.01em" }}>Castor</div>
        </div>
      </div>
      <Sfx at={8} src="pencil.wav" vol={0.9} />
      <Sfx at={74} src="gnaw.wav" vol={0.8} />
      <Sfx at={112} src="tick.wav" vol={0.5} />
    </AbsoluteFill>
  );
};
