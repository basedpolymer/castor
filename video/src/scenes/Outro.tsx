// 3 (end) CLI + MCP, 4. Free · offline · open source in numbers, 5. the address, the mark,
// and a crumb that falls off the diamond.
import React from "react";
import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
import { Mark, prog } from "../components/Mark";
import { Caption, SectionTitle, useLayout } from "../components/Page";
import { Lang, T } from "../i18n";
import { C, F } from "../theme";
import { clamp, io, Rise, Sfx, Typing, typed, useSceneFade } from "./common";

export const Cli: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v, m } = useLayout();
  const t = T[lang];
  const fade = useSceneFade(dur);
  const EVERY = 1;
  const A = 14;
  const outAt = A + t.cliCmd.length * EVERY + 10;
  const B = outAt + 26;
  const fs = v ? 26 : 30;
  const line = (prompt: boolean, text: string, color: string = C.ink) => (
    <div style={{ whiteSpace: v ? "pre-wrap" : "pre", color, minHeight: "1.6em", paddingLeft: v && !prompt ? "1.2em" : 0, textIndent: v && !prompt ? "-1.2em" : 0 }}>
      {prompt ? <span style={{ color: C.muted }}>$ </span> : null}
      {text}
    </div>
  );
  const caret = Math.floor(f / 15) % 2 === 0;
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <div style={{ position: "absolute", left: m, right: m, top: v ? 300 : 220 }}>
        <Rise at={0}>
          <div style={{ font: `700 ${v ? 44 : 40}px ${F.sans}`, color: C.ink, marginBottom: 10 }}>{lang === "fr" ? "Terminal et MCP" : "CLI and MCP"}</div>
          <div style={{ font: `400 ${v ? 30 : 28}px/1.4 ${F.roman}`, color: C.muted, marginBottom: 34 }}>
            {lang === "fr" ? "Les mêmes outils, le même historique, les mêmes réglages." : "The same tools, the same history, the same settings."}
          </div>
        </Rise>
        <div
          style={{
            padding: v ? "26px 26px" : "30px 36px",
            borderLeft: `3px solid ${C.faint}`,
            background: C.surface,
            font: `400 ${fs}px/1.6 ${F.mono}`,
            overflow: "hidden",
          }}
        >
          {line(true, typed(t.cliCmd, f, A, EVERY) + (f < outAt && caret ? "▏" : ""))}
          <div style={{ opacity: io(f, outAt, outAt + 6) }}>
            {line(false, t.cliOut[0].replace(/^ {2}/, v ? "" : "  "), C.ink)}
            {line(false, t.cliOut[1].replace(/^ {4}/, v ? "  " : "    "), C.good)}
          </div>
          <div style={{ height: 18 }} />
          {f >= B ? line(true, typed(t.mcpCmd, f, B, EVERY) + (caret ? "▏" : "")) : line(false, "")}
        </div>
        {/* the MCP server's tools, as in docs/MCP.md */}
        <div style={{ marginTop: v ? 40 : 34, opacity: io(f, B + t.mcpCmd.length + 6, B + t.mcpCmd.length + 20) }}>
          <div style={{ font: `400 ${v ? 28 : 26}px ${F.roman}`, color: C.muted, marginBottom: 14 }}>
            {lang === "fr" ? "Ce que voit l’agent IA :" : "What the AI agent sees:"}
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: v ? "12px 14px" : "12px 16px", font: `400 ${v ? 26 : 26}px ${F.mono}` }}>
            {["list_tools", "describe_tool", "run_tool", "run_chain", "history"].map((n) => (
              <span key={n} style={{ padding: "8px 16px", border: `1.5px solid ${C.rule}`, borderRadius: 4, background: C.raised, color: C.ink }}>
                {n}
              </span>
            ))}
          </div>
        </div>
      </div>
      <Caption label={t.figure} n={10} text={t.figCli} url={t.url} />
      <Typing from={A} count={t.cliCmd.length} every={EVERY * 2} vol={0.35} />
      <Sfx at={outAt} src="tick.wav" vol={0.4} />
      <Typing from={B} count={t.mcpCmd.length} every={EVERY * 2} vol={0.35} />
    </AbsoluteFill>
  );
};

export const Numbers: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v, m } = useLayout();
  const t = T[lang];
  const fade = useSceneFade(dur);
  const tools = Math.round(interpolate(f, [30, 62], [0, 110], { ...clamp, easing: (x) => 1 - Math.pow(1 - x, 3) }));
  const row = (i: number) => io(f, 26 + i * 16, 42 + i * 16);
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <div style={{ position: "absolute", left: m, right: m, top: v ? 300 : 190 }}>
        <Rise at={0}>
          <SectionTitle num="4." title={t.sec4} size={v ? 64 : 72} ghost={lang === "fr" ? "G" : "F"} />
        </Rise>
        <div style={{ marginTop: v ? 90 : 70, width: "100%" }}>
          <Rise at={12}>
            <div style={{ font: `400 ${v ? 30 : 28}px ${F.roman}`, color: C.ink, marginBottom: 14 }}>
              <b style={{ fontFamily: F.sans }}>{t.table} 1</b> — <span style={{ color: C.muted }}>{t.tableCap}</span>
            </div>
          </Rise>
          <div
            style={{
              borderTop: `2.4px solid ${C.ink}`,
              borderBottom: `2.4px solid ${C.ink}`,
              opacity: io(f, 14, 26),
              padding: "12px 0",
            }}
          >
            {t.rows.map(([k, val], i) => (
              <div
                key={k}
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "baseline",
                  padding: v ? "16px 8px" : "14px 12px",
                  borderTop: i === 0 ? undefined : `1px solid ${C.rule}`,
                  opacity: row(i),
                }}
              >
                <span style={{ font: `400 ${v ? 40 : 40}px ${F.sans}`, color: C.muted }}>{k}</span>
                <span style={{ font: `700 ${v ? 76 : 76}px/1 ${F.sans}`, color: C.ink, fontVariantNumeric: "tabular-nums" }}>{i === 0 ? tools : val}</span>
              </div>
            ))}
          </div>
          <Rise at={86}>
            <div style={{ marginTop: 26, font: `400 ${v ? 32 : 30}px ${F.sans}`, color: C.muted }}>{t.licence}</div>
          </Rise>
        </div>
      </div>
      {[0, 1, 2].map((i) => (
        <Sfx key={i} at={30 + i * 16} src="tick.wav" vol={0.4} />
      ))}
    </AbsoluteFill>
  );
};

export const End: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v, H } = useLayout();
  const t = T[lang];
  const fade = useSceneFade(dur, 14, 1);
  const size = v ? 300 : 230;
  const LET_GO = 150; // the crumb lets go
  const fall = Math.max(0, (f - LET_GO) / 30);
  return (
    <AbsoluteFill style={{ opacity: fade, alignItems: "center", justifyContent: "center", textAlign: "center" }}>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", transform: `translateY(${v ? -40 : -20}px)` }}>
        <Rise at={4}>
          <Mark id="end" size={size} fall={fall} fallTo={H} draw={1} fill={1} bite={1} letter={1} crumbs={1} />
        </Rise>
        <Rise at={20} style={{ marginTop: v ? 70 : 46 }}>
          <div style={{ font: `700 ${v ? 70 : 84}px/1.1 ${F.sans}`, color: C.ink, letterSpacing: "-0.01em" }}>{t.url}</div>
        </Rise>
        <Rise at={34} style={{ marginTop: v ? 44 : 34 }}>
          <div style={{ display: "flex", flexDirection: v ? "column" : "row", gap: v ? 18 : 18, font: `700 ${v ? 36 : 30}px ${F.sans}` }}>
            {["macOS", "Windows 10 / 11"].map((p) => (
              <span key={p} style={{ padding: v ? "18px 40px" : "14px 30px", borderRadius: 4, background: C.ink, color: C.paper }}>
                <small style={{ display: "block", fontWeight: 400, fontSize: "0.62em", opacity: 0.85 }}>{t.download}</small>
                {p}
              </span>
            ))}
          </div>
        </Rise>
        <Rise at={48} style={{ marginTop: v ? 40 : 32 }}>
          <div style={{ font: `400 ${v ? 32 : 28}px ${F.sans}`, color: C.muted }}>{t.licence}</div>
        </Rise>
        <Rise at={60} style={{ marginTop: v ? 30 : 22 }}>
          <div style={{ font: `400 ${v ? 34 : 30}px ${F.slant}`, color: C.ink, maxWidth: v ? 900 : undefined }}>{t.joke}</div>
        </Rise>
      </div>
      <Sfx at={LET_GO} src="crumb.wav" vol={0.8} />
    </AbsoluteFill>
  );
};

export { prog };
