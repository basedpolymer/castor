// 2. "Gnaws through your files." — the tool cards cascade in, family by family.
import React from "react";
import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
import tools from "../data/tools.json";
import { Caption, Diamond, useLayout } from "../components/Page";
import { Lang, T } from "../i18n";
import { C, F, FAM, FAMILIES, Family } from "../theme";
import { clamp, io, Rise, Sfx, useSceneFade } from "./common";

type Tool = { id: string; family: Family; name: { en: string; fr: string } };
const ALL = tools as Tool[];
// the best-known tools of each family first, the rest in catalogue order
const FIRST: Record<Family, string[]> = {
  pdf: ["compress", "merge", "ocr", "redact", "sign", "split", "organize"],
  image: ["remove-background", "compress", "upscale", "convert", "resize", "vectorize", "watermark"],
  video: ["trim", "compress", "captions", "to-gif", "convert", "remove-silences", "join"],
  audio: ["transcribe", "denoise", "normalize", "trim", "compress", "convert", "merge"],
  create: ["qr-code", "mockup", "favicon", "social-card", "gradient", "barcode", "formula"],
  files: ["rename", "archive", "duplicates", "identify", "sort", "checksum", "dates"],
};
const byFamily = (fam: Family) => {
  const list = ALL.filter((t) => t.family === fam);
  const first = FIRST[fam].map((id) => list.find((t) => t.id === `${fam}.${id}`)!).filter(Boolean);
  return { first, total: list.length };
};

const Card: React.FC<{ name: string; fam: Family; p: number; h: number; fs: number }> = ({ name, fam, p, h, fs }) => (
  <div
    style={{
      position: "relative",
      height: h,
      display: "flex",
      alignItems: "center",
      gap: 12,
      padding: "0 14px 0 20px",
      background: C.raised,
      border: `1.5px solid ${C.rule}`,
      borderRadius: 4,
      overflow: "hidden",
      opacity: p,
      transform: `translateY(${(1 - p) * -26}px)`,
      font: `400 ${fs}px ${F.sans}`,
      color: C.ink,
      whiteSpace: "nowrap",
    }}
  >
    <span style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: 7, background: FAM[fam].tint }} />
    <Diamond size={fs * 0.44} color={FAM[fam].stroke} stroke={2.2} />
    <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{name}</span>
  </div>
);

export const Cascade: React.FC<{ dur: number; lang: Lang }> = ({ dur, lang }) => {
  const f = useCurrentFrame();
  const { v, m, W } = useLayout();
  const t = T[lang];
  const fade = useSceneFade(dur);
  const perFam = v ? 3 : 6;
  const start = 70; // cards start falling
  const colW = v ? (W - 2 * m - 30) / 2 : (W - 2 * m - 5 * 22) / 6;
  const cardH = v ? 58 : 52;
  const fs = v ? 25 : 20;
  // each family column after the previous one; inside a column, cards 5 frames apart
  const at = (col: number, i: number) => start + col * 11 + i * 5;
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <div style={{ position: "absolute", left: m, top: v ? 210 : 150, right: m }}>
        <Rise at={4}>
          <div style={{ font: `400 ${v ? 70 : 78}px/1.12 ${F.sans}`, color: C.ink }}>{t.tagline1}</div>
        </Rise>
        <Rise at={26}>
          <div style={{ font: `400 ${v ? 70 : 78}px/1.12 ${F.sans}`, color: C.muted, marginTop: 6 }}>{t.tagline2}</div>
        </Rise>
      </div>
      <div
        style={{
          position: "absolute",
          left: m,
          right: m,
          top: v ? 540 : 420,
          display: "grid",
          gridTemplateColumns: `repeat(${v ? 2 : 6}, ${colW}px)`,
          columnGap: v ? 30 : 22,
          rowGap: v ? 44 : 0,
        }}
      >
        {FAMILIES.map((fam, col) => {
          const { first, total } = byFamily(fam);
          const shown = first.slice(0, perFam);
          const head = io(f, at(col, 0) - 8, at(col, 0) + 6);
          const more = io(f, at(col, perFam) + 2, at(col, perFam) + 14);
          return (
            <div key={fam} style={{ display: "flex", flexDirection: "column", gap: v ? 12 : 10 }}>
              <div
                style={{
                  display: "flex",
                  alignItems: "baseline",
                  justifyContent: "space-between",
                  font: `700 ${v ? 32 : 26}px ${F.sans}`,
                  color: C.ink,
                  opacity: head,
                  paddingBottom: 6,
                  borderBottom: `1.5px solid ${C.ink}`,
                  marginBottom: 6,
                }}
              >
                <span>
                  <span style={{ marginRight: "0.4em" }}>{col + 1}.</span>
                  {t.fam[fam]}
                </span>
                <span style={{ fontVariantNumeric: "tabular-nums" }}>{total}</span>
              </div>
              {shown.map((tool, i) => (
                <Card key={tool.id} name={tool.name[lang]} fam={fam} p={io(f, at(col, i), at(col, i) + 12)} h={cardH} fs={fs} />
              ))}
              <div style={{ font: `400 ${v ? 26 : 21}px ${F.roman}`, color: C.muted, opacity: more, marginTop: 4 }}>
                {lang === "fr" ? `et ${total - perFam} de plus…` : `and ${total - perFam} more…`}
              </div>
            </div>
          );
        })}
      </div>
      <Caption label={t.figure} n={1} text={t.toolsLine + "."} url={t.url} opacity={interpolate(f, [start + 40, start + 60], [0, 1], clamp)} />
      {FAMILIES.map((_, col) => (
        <Sfx key={col} at={at(col, 0)} src="tick.wav" vol={0.45} />
      ))}
    </AbsoluteFill>
  );
};
