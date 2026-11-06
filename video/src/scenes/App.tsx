// 3. The app at work, from real screenshots: drop → Compress PDF with the real sizes, remove a
// background, Trim & Caption Studio, the ⌘K palette, the menu-bar quick panel; then chains and runs.
import React from "react";
import { AbsoluteFill, Img, interpolate, staticFile, useCurrentFrame } from "remotion";
import { Caption, useLayout } from "../components/Page";
import { Box, CamKey, Cursor, FileChip, Highlight, Rect, Screen } from "../components/Screen";
import { Lang, quote, T } from "../i18n";
import { C, F } from "../theme";
import { clamp, io, Sfx, Typing, typed, useSceneFade } from "./common";
import { GEO, PANEL, RMBG, SHOT } from "./geometry";

const shot = (lang: Lang, name: string, size = SHOT) => ({ src: `shots/${lang}/${name}.png`, w: size.w, h: size.h });
const FULL: Rect = [0, 0, SHOT.w, SHOT.h];

export const useBox = (): Box => {
  const { v } = useLayout();
  // 16:9: the box has the shots' 1.6 ratio, so the full view fills it exactly
  return v ? { x: 40, y: 200, w: 1000, h: 1380 } : { x: 279, y: 126, w: 1363, h: 852 };
};

const center = (r: Rect) => ({ x: r[0] + r[2] / 2, y: r[1] + r[3] / 2 });

/** Caption that switches text at `switchAt` (cross-fade). */
const Captions: React.FC<{ items: { from: number; n: number; text: string }[]; lang: Lang }> = ({ items, lang }) => {
  const f = useCurrentFrame();
  const t = T[lang];
  return (
    <>
      {items.map((it, i) => {
        const next = items[i + 1]?.from ?? Infinity;
        const o = Math.min(
          interpolate(f, [it.from, it.from + 10], [0, 1], clamp),
          Number.isFinite(next) ? interpolate(f, [next - 4, next + 4], [1, 0], clamp) : 1,
        );
        return o > 0 ? <Caption key={i} label={t.figure} n={it.n} text={it.text} url={t.url} opacity={o} /> : null;
      })}
    </>
  );
};

// ------------------------------------------------------------------------------------------
export const Compress: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v } = useLayout();
  const box = useBox();
  const g = GEO[lang];
  const t = T[lang];
  const fade = useSceneFade(dur);
  const drop = center(g.dropZone);
  const card = center(g.suggestedCompress);
  const btn = center(g.compressButton);
  const T_DROP = 60;
  const T_CARD = 106;
  const T_BTN = 150;
  const T_DONE = 198;
  const tool: Rect = [236, 28, 1140, 580];
  const keys: CamKey[] = v
    ? [
        { f: 0, r: [180, 0, 1000, 600] },
        { f: T_DROP + 6, r: [180, 60, 1000, 600] },
        { f: 92, r: [250, 140, 560, 620] },
        { f: T_CARD + 8, r: [250, 140, 560, 620] },
        { f: 132, r: [240, 150, 760, 520] },
        { f: T_BTN - 12, r: [700, 180, 690, 560] },
        { f: T_BTN + 10, r: [700, 180, 690, 560] },
        { f: T_DONE - 14, r: [250, 160, 700, 560] },
        { f: 232, r: [252, 220, 620, 260] },
      ]
    : [
        { f: 0, r: FULL },
        { f: T_DROP + 6, r: FULL },
        { f: 92, r: [240, 140, 1130, 380] },
        { f: T_CARD + 8, r: [240, 140, 1130, 380] },
        { f: 132, r: tool },
        { f: T_DONE + 2, r: tool },
        { f: 232, r: [250, 170, 760, 420] },
      ];
  const cursor = [
    { f: 10, x: 1240, y: 820 },
    { f: T_DROP - 4, x: drop.x - 40, y: drop.y },
    { f: T_DROP + 8, x: drop.x - 40, y: drop.y + 6 },
    { f: T_CARD - 6, x: card.x, y: card.y },
    { f: T_CARD + 18, x: card.x + 10, y: card.y + 10 },
    { f: T_BTN - 6, x: btn.x, y: btn.y },
    { f: T_BTN + 30, x: btn.x + 30, y: btn.y + 120 },
  ];
  const carrying = f < T_DROP + 2;
  const names = lang === "fr" ? ["scan-chapitre-1.pdf", "scan-chapitre-2.pdf"] : ["scan-chapter-1.pdf", "scan-chapter-2.pdf"];
  const sizes = lang === "fr" ? ["4,8 Mo", "3,3 Mo"] : ["4.8 MB", "3.3 MB"];
  const prog = (row: number) => interpolate(f, [T_BTN + 4 + row * 6, T_DONE - 6 - (1 - row) * 8], [0, 1], clamp);
  const big = io(f, 238, 258);
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <Screen
        box={box}
        keys={keys}
        layers={[
          shot(lang, "home"),
          { ...shot(lang, "home-drop"), at: T_DROP + 2, dur: 8 },
          { ...shot(lang, "pdf-queued"), at: T_CARD + 6, dur: 8 },
          { ...shot(lang, "pdf-done"), at: T_DONE, dur: 6 },
        ]}
        overlay={(z, fr) => (
          <>
            {fr < T_DONE + 6 && fr > T_BTN
              ? g.statusCells.map((r, i) => (
                  <div key={i} style={{ position: "absolute", left: r[0], top: r[1], width: r[2], height: r[3], background: g.rowBg[i] }}>
                    <div style={{ position: "absolute", left: 2, top: r[3] / 2 - 3, width: r[2] - 30, height: 6, background: C.rule, borderRadius: 3 }} />
                    <div style={{ position: "absolute", left: 2, top: r[3] / 2 - 3, width: (r[2] - 30) * prog(i), height: 6, background: C.ink, borderRadius: 3 }} />
                  </div>
                ))
              : null}
            <Highlight r={g.dropZone} z={z} at={T_DROP - 22} f={fr} out={T_DROP + 2} pad={4} />
            <Highlight r={g.suggestedCompress} z={z} at={T_CARD - 14} f={fr} out={T_CARD + 4} pad={4} />
            <Highlight r={g.doneLine} z={z} at={222} f={fr} pad={5} />
            <Highlight r={g.doneJoke} z={z} at={244} f={fr} pad={5} />
            <Cursor
              keys={cursor}
              clicks={[T_CARD, T_BTN]}
              z={z}
              f={fr}
              from={10}
              to={T_BTN + 30}
              carry={carrying ? (
                <div style={{ position: "relative" }}>
                  <FileChip name={names[1]} size={sizes[1]} offset={1} />
                  <FileChip name={names[0]} size={sizes[0]} />
                </div>
              ) : undefined}
            />
          </>
        )}
      />
      {/* the result, set large: real sizes from this run */}
      <div
        style={{
          position: "absolute",
          right: v ? 40 : 279 + 60,
          left: v ? 40 : undefined,
          top: v ? 1150 : 700,
          padding: v ? "30px 40px 34px" : "26px 40px 30px",
          background: C.raised,
          border: `1.5px solid ${C.rule}`,
          borderTop: `2.4px solid ${C.ink}`,
          borderBottom: `2.4px solid ${C.ink}`,
          boxShadow: "0 18px 50px rgba(27,22,22,.12)",
          opacity: big,
          transform: `translateY(${(1 - big) * 16}px)`,
        }}
      >
        <div style={{ display: "flex", alignItems: "baseline", gap: 26, font: `700 ${v ? 84 : 80}px/1 ${F.sans}`, color: C.ink, whiteSpace: "nowrap" }}>
          <span>{t.bigBefore}</span>
          <span style={{ fontWeight: 400, color: C.muted }}>→</span>
          <span>{t.bigAfter}</span>
          <span style={{ color: C.good, fontSize: "0.62em", marginLeft: 8 }}>{t.bigPct}</span>
        </div>
        <div style={{ marginTop: 14, font: `400 ${v ? 30 : 27}px ${F.roman}`, color: C.muted }}>{t.bigNote}</div>
      </div>
      <Captions
        lang={lang}
        items={[
          { from: 0, n: 2, text: t.figDrop },
          { from: T_CARD + 6, n: 3, text: t.figCompress },
        ]}
      />
      <Sfx at={T_DROP} src="drop.wav" vol={0.8} />
      <Sfx at={T_CARD} src="click.wav" vol={0.7} />
      <Sfx at={T_BTN} src="click.wav" vol={0.7} />
      <Sfx at={T_DONE} src="chime.wav" vol={0.6} />
    </AbsoluteFill>
  );
};

// ------------------------------------------------------------------------------------------
export const Rmbg: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const { v } = useLayout();
  const box = useBox();
  const t = T[lang];
  const fade = useSceneFade(dur);
  const keys: CamKey[] = v
    ? [
        { f: 0, r: [250, 170, 750, 300] },
        { f: 40, r: [250, 170, 750, 300] },
        { f: 95, r: [255, 640, 740, 420] },
      ]
    : [
        { f: 0, r: [240, 30, 1140, 640] },
        { f: 40, r: [240, 30, 1140, 640] },
        { f: 95, r: [240, 600, 1140, 470] },
      ];
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <Screen box={box} keys={keys} layers={[shot(lang, "rmbg-done", RMBG)]} maxZoom={1.6} />
      <Captions lang={lang} items={[{ from: 0, n: 4, text: t.figRmbg }]} />
    </AbsoluteFill>
  );
};

// ------------------------------------------------------------------------------------------
export const Studio: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const { v } = useLayout();
  const box = useBox();
  const t = T[lang];
  const fade = useSceneFade(dur);
  const timeline: Rect = [216, 668, 1184, 150];
  const cuts = GEO[lang].studioCuts;
  const c1 = cuts[0][0] + cuts[0][2] / 2;
  const c2 = cuts[1][0] + cuts[1][2] / 2;
  const keys: CamKey[] = v
    ? [
        { f: 0, r: [c1 - 250, 600, 700, 230] },
        { f: 30, r: [c1 - 250, 600, 700, 230] },
        { f: 62, r: [c2 - 400, 600, 800, 230] },
        { f: 78, r: [232, 90, 840, 560] },
        { f: 110, r: [1080, 90, 320, 600] },
      ]
    : [
        { f: 0, r: [216, 36, 1184, 839] },
        { f: 22, r: [216, 36, 1184, 839] },
        { f: 50, r: [216, 560, 1184, 280] },
        { f: 74, r: [216, 560, 1184, 280] },
        { f: 98, r: [216, 36, 1184, 650] },
      ];
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <Screen
        box={box}
        keys={keys}
        layers={[shot(lang, "studio-cut"), { ...shot(lang, "studio-captions"), at: 76, dur: 10 }]}
        overlay={(z, fr) => (
          <>
            {cuts.map((r, i) => (
              <Highlight key={i} r={r} z={z} at={36 + i * 8} f={fr} out={76} pad={2} />
            ))}
            <Highlight r={timeline} z={z} at={-100} f={fr} out={-90} />
          </>
        )}
      />
      <Captions lang={lang} items={[{ from: 0, n: 5, text: t.figStudio }]} />
      <Sfx at={36} src="tick.wav" vol={0.4} />
      <Sfx at={44} src="tick.wav" vol={0.4} />
    </AbsoluteFill>
  );
};

// ------------------------------------------------------------------------------------------
export const Palette: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v } = useLayout();
  const box = useBox();
  const g = GEO[lang];
  const t = T[lang];
  const fade = useSceneFade(dur);
  const OPEN = 22;
  const TYPE = 32;
  const EVERY = 2;
  const n = [...t.query].length;
  const shown = typed(t.query, f, TYPE, EVERY).length;
  const r = g.paletteText;
  const revealW = (r[2] * shown) / n;
  const keys: CamKey[] = v
    ? [
        { f: 0, r: [300, 0, 700, 300] },
        { f: OPEN, r: [390, 60, 620, 420] },
      ]
    : [
        { f: 0, r: FULL },
        { f: OPEN + 4, r: [330, 40, 740, 470] },
      ];
  const key = io(f, 4, 12) * (1 - io(f, OPEN + 6, OPEN + 14));
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <Screen
        box={box}
        keys={keys}
        layers={[shot(lang, "home"), { ...shot(lang, "palette"), at: OPEN, dur: 5 }]}
        overlay={(z, fr) => (
          <>
            {fr >= OPEN && shown < n ? (
              <div style={{ position: "absolute", left: r[0] + revealW, top: r[1], width: r[2] - revealW + 4, height: r[3], background: g.fieldBg }}>
                <div style={{ position: "absolute", left: 0, top: 2, width: 1.2, height: r[3] - 4, background: C.ink }} />
              </div>
            ) : null}
            <Highlight r={g.paletteRow} z={z} at={TYPE + n * EVERY + 6} f={fr} pad={3} />
          </>
        )}
      />
      {/* the shortcut, as keycaps */}
      <div
        style={{
          position: "absolute",
          left: "50%",
          top: v ? 820 : 470,
          transform: `translate(-50%, ${(1 - key) * 10}px)`,
          opacity: key,
          display: "flex",
          gap: 14,
          font: `400 ${v ? 54 : 48}px ${F.sans}`,
          color: C.ink,
        }}
      >
        {["⌘", "K"].map((k) => (
          <span key={k} style={{ minWidth: 76, padding: "10px 20px", textAlign: "center", background: C.raised, border: `1.5px solid ${C.rule}`, borderBottom: `4px solid ${C.rule}`, borderRadius: 8 }}>
            {k}
          </span>
        ))}
        <span style={{ alignSelf: "center", font: `400 ${v ? 34 : 30}px ${F.roman}`, color: C.muted, marginLeft: 8 }}>
          {lang === "fr" ? "ou Ctrl K" : "or Ctrl K"}
        </span>
      </div>
      <Captions
        lang={lang}
        items={[
          {
            from: 0,
            n: 6,
            text:
              lang === "fr"
                ? `La palette comprend : ${quote(lang, t.query)} trouve Compresser un PDF.`
                : `${t.figPalette} ${quote(lang, t.query)} finds Compress PDF.`,
          },
        ]}
      />
      <Sfx at={6} src="key1.wav" vol={0.7} />
      <Sfx at={9} src="key2.wav" vol={0.7} />
      <Typing from={TYPE} count={n} every={EVERY} vol={0.45} />
    </AbsoluteFill>
  );
};

// ------------------------------------------------------------------------------------------
export const Panel: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v, W, m } = useLayout();
  const g = GEO[lang];
  const t = T[lang];
  const fade = useSceneFade(dur);
  const barH = v ? 52 : 40;
  const barTop = v ? 200 : 126;
  const barL = v ? 40 : 279;
  const barR = v ? W - 40 : 1641;
  const s = v ? 1.9 : 1.42; // panel scale
  const pw = PANEL.w * s;
  const px = v ? (W - pw) / 2 : barR - 40 - pw;
  const iconX = px + pw / 2 - 12; // the panel opens under its icon
  const OPEN = 26;
  const open = io(f, OPEN, OPEN + 12);
  const py = barTop + barH + 10;
  const clip = g.clipboardFirst;
  const target = { x: px + (clip[0] + clip[2] / 2) * s, y: py + (clip[1] + clip[3] / 2) * s };
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      {/* a plain desktop: a menu bar with Castor's mark */}
      <div
        style={{
          position: "absolute",
          left: barL,
          width: barR - barL,
          top: barTop,
          height: barH,
          background: C.surface,
          border: `1.5px solid ${C.rule}`,
          borderRadius: 8,
          display: "flex",
          alignItems: "center",
          justifyContent: "flex-end",
          gap: 26,
          padding: "0 22px",
          font: `400 ${v ? 26 : 20}px ${F.sans}`,
          color: C.muted,
        }}
      >
        <span style={{ position: "absolute", left: iconX - barL - 14, top: 4, width: barH - 8 + 4, height: barH - 8, borderRadius: 5, background: f >= OPEN - 2 ? C.rule : "transparent" }} />
        <Img src={staticFile("brand/castor-tray.svg")} style={{ position: "absolute", left: iconX - barL, top: (barH - (v ? 30 : 24)) / 2, width: v ? 30 : 24, height: v ? 30 : 24 }} />
        <span>{lang === "fr" ? "ven. 6 nov." : "Fri 6 Nov"}</span>
        <span>10:24</span>
      </div>
      <div
        style={{
          position: "absolute",
          left: px,
          top: py,
          width: pw,
          height: PANEL.h * s,
          opacity: open,
          transform: `translateY(${(1 - open) * -12}px)`,
          borderRadius: 10,
          overflow: "hidden",
          border: `1.5px solid ${C.rule}`,
          boxShadow: "0 24px 60px rgba(27,22,22,.16)",
        }}
      >
        <div style={{ position: "absolute", left: 0, top: 0, width: PANEL.w, height: PANEL.h, transform: `scale(${s})`, transformOrigin: "0 0" }}>
          <Img src={staticFile(`shots/${lang}/panel.png`)} style={{ width: PANEL.w, height: PANEL.h }} />
          <Highlight r={g.clipboard} z={s} at={OPEN + 22} f={f} pad={2} />
        </div>
      </div>
      {/* the image on the clipboard, as a small thumbnail beside the panel */}
      <div
        style={{
          position: "absolute",
          left: v ? (W - 380) / 2 : 279 + 60,
          top: v ? py + PANEL.h * s + 36 : py + 150,
          width: v ? 380 : 560,
          opacity: io(f, OPEN + 22, OPEN + 36),
          font: `400 ${v ? 28 : 24}px/1.35 ${F.roman}`,
          color: C.muted,
        }}
      >
        <Img src={staticFile("clipboard.png")} style={{ width: "100%", border: `1.5px solid ${C.rule}`, borderRadius: 4, display: "block" }} />
        <div style={{ marginTop: 10, whiteSpace: "nowrap" }}>
          <b style={{ fontFamily: F.sans, color: C.ink }}>{lang === "fr" ? "Presse-papiers" : "Clipboard"}</b> · PNG 1280 × 800
        </div>
      </div>
      <div style={{ position: "absolute", inset: 0 }}>
        <Cursor
          keys={[
            { f: 4, x: iconX - 120, y: barTop + 260 },
            { f: OPEN - 4, x: iconX + 12, y: barTop + barH / 2 },
            { f: OPEN + 30, x: iconX + 12, y: barTop + barH / 2 + 20 },
            { f: 82, x: target.x, y: target.y },
          ]}
          clicks={[OPEN - 2, 86]}
          z={1}
          f={f}
          from={4}
          to={dur}
        />
      </div>
      <Captions lang={lang} items={[{ from: 0, n: 7, text: t.figPanel }]} />
      <Sfx at={OPEN - 2} src="click.wav" vol={0.7} />
      <Sfx at={86} src="click.wav" vol={0.7} />
    </AbsoluteFill>
  );
};

// ------------------------------------------------------------------------------------------
export const Chain: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const { v } = useLayout();
  const box = useBox();
  const g = GEO[lang];
  const t = T[lang];
  const fade = useSceneFade(dur);
  const keys: CamKey[] = v
    ? [
        { f: 0, r: [250, 60, 330, 560] },
        { f: 26, r: [250, 60, 330, 560] },
        { f: 56, r: [580, 150, 560, 520] },
        { f: 96, r: [760, 150, 560, 520] },
      ]
    : [
        { f: 0, r: FULL },
        { f: 24, r: FULL },
        { f: 54, r: [250, 140, 1110, 560] },
      ];
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <Screen
        box={box}
        keys={keys}
        layers={[shot(lang, "chain-scans")]}
        overlay={(z, fr) => (
          <>
            {g.chainSteps.map((r, i) => (
              <Highlight key={i} r={r} z={z} at={56 + i * 16} f={fr} pad={4} />
            ))}
          </>
        )}
      />
      <Captions lang={lang} items={[{ from: 0, n: 8, text: t.figChain }]} />
      {[0, 1, 2].map((i) => (
        <Sfx key={i} at={56 + i * 16} src="tick.wav" vol={0.4} />
      ))}
    </AbsoluteFill>
  );
};

// ------------------------------------------------------------------------------------------
export const Runs: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v } = useLayout();
  const box = useBox();
  const g = GEO[lang];
  const t = T[lang];
  const fade = useSceneFade(dur);
  const keys: CamKey[] = v
    ? [
        { f: 0, r: [260, 200, 640, 300] },
        { f: 84, r: [260, 200, 640, 300] },
        { f: 112, r: [760, 200, 640, 300] },
      ]
    : [
        { f: 0, r: FULL },
        { f: 20, r: FULL },
        { f: 50, r: [240, 60, 1140, 460] },
      ];
  const note = io(f, 62, 76);
  const r = g.renameRow;
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <Screen
        box={box}
        keys={keys}
        layers={[shot(lang, "runs")]}
        overlay={(z, fr) => (
          <>
            <Highlight r={r} z={z} at={50} f={fr} pad={4} />
            <div
              style={{
                position: "absolute",
                left: r[0] + 110,
                top: r[1] + r[3] + 14,
                padding: "8px 14px",
                background: C.raised,
                border: `1px solid ${C.ink}`,
                borderRadius: 4,
                font: `400 16px ${F.roman}`,
                color: C.ink,
                opacity: note,
                transform: `translateY(${(1 - note) * 6}px)`,
                whiteSpace: "nowrap",
              }}
            >
              <span style={{ fontFamily: F.sans, marginRight: 8 }}>↶</span>
              {t.undo}
            </div>
          </>
        )}
      />
      <Captions lang={lang} items={[{ from: 0, n: 9, text: t.figRuns }]} />
      <Sfx at={50} src="tick.wav" vol={0.4} />
    </AbsoluteFill>
  );
};
