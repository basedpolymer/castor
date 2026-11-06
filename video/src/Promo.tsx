// The whole promo: one timeline, two languages, two formats (16:9 and 9:16).
import React from "react";
import { AbsoluteFill, interpolate, Sequence, useCurrentFrame } from "remotion";
import { Paper, RunningHead } from "./components/Page";
import { Lang, T } from "./i18n";
import { Chain, Compress, Palette, Panel, Rmbg, Runs, Studio } from "./scenes/App";
import { Cascade } from "./scenes/Cascade";
import { clamp } from "./scenes/common";
import { Intro } from "./scenes/Intro";
import { Cli, End, Numbers } from "./scenes/Outro";
import { TL } from "./timeline";
export { FPS, TL, TOTAL } from "./timeline";

export type PromoProps = { lang: Lang };

const Head: React.FC<{ lang: Lang }> = ({ lang }) => {
  const f = useCurrentFrame();
  const t = T[lang];
  const sections: [number, number, string][] = [
    [TL.cascade[0], TL.compress[0] + 6, `1.  ${t.sec1}`],
    [TL.compress[0] + 6, TL.chain[0] + 6, `2.  ${t.sec2}`],
    [TL.chain[0] + 6, TL.numbers[0] + 6, `3.  ${t.sec3}`],
    [TL.numbers[0] + 6, TL.end[0] + 6, `4.  ${t.sec4}`],
  ];
  const all = Math.min(interpolate(f, [TL.cascade[0], TL.cascade[0] + 14], [0, 1], clamp), interpolate(f, [TL.end[0], TL.end[0] + 14], [1, 0], clamp));
  if (all <= 0) return null;
  return (
    <>
      {sections.map(([a, b, label]) => {
        const o = Math.min(interpolate(f, [a - 6, a + 6], [0, 1], clamp), interpolate(f, [b - 6, b + 6], [1, 0], clamp));
        return o > 0 ? <RunningHead key={label} label={label} num="110" opacity={o * all} /> : null;
      })}
    </>
  );
};

export const Promo: React.FC<PromoProps> = ({ lang }) => {
  const seq = (k: keyof typeof TL, el: (dur: number) => React.ReactNode) => (
    <Sequence key={k} from={TL[k][0]} durationInFrames={TL[k][1]} name={k}>
      {el(TL[k][1])}
    </Sequence>
  );
  return (
    <AbsoluteFill>
      <Paper />
      {seq("intro", (d) => <Intro dur={d} />)}
      {seq("cascade", (d) => <Cascade dur={d} lang={lang} />)}
      {seq("compress", (d) => <Compress dur={d} lang={lang} />)}
      {seq("rmbg", (d) => <Rmbg dur={d} lang={lang} />)}
      {seq("studio", (d) => <Studio dur={d} lang={lang} />)}
      {seq("palette", (d) => <Palette dur={d} lang={lang} />)}
      {seq("panel", (d) => <Panel dur={d} lang={lang} />)}
      {seq("chain", (d) => <Chain dur={d} lang={lang} />)}
      {seq("runs", (d) => <Runs dur={d} lang={lang} />)}
      {seq("cli", (d) => <Cli dur={d} lang={lang} />)}
      {seq("numbers", (d) => <Numbers dur={d} lang={lang} />)}
      {seq("end", (d) => <End dur={d} lang={lang} />)}
      <Head lang={lang} />
    </AbsoluteFill>
  );
};
