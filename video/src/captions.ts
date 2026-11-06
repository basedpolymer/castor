// WebVTT captions for the 16:9 versions, from the same timeline and strings as the video.
// There is no voice: the captions carry the on-screen text, and say what the pictures show.
import { Lang, quote, T } from "./i18n";
import { FPS, TL } from "./timeline";

const ts = (frame: number) => {
  const s = frame / FPS;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = (s % 60).toFixed(3).padStart(6, "0");
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${sec}`;
};

export const vtt = (lang: Lang) => {
  const t = T[lang];
  const fr = lang === "fr";
  const end = (k: keyof typeof TL) => TL[k][0] + TL[k][1] - 6;
  const start = (k: keyof typeof TL, d = 6) => TL[k][0] + d;
  const cues: [number, number, string][] = [
    [0, end("intro"), fr ? "[Un losange se dessine, puis se fait grignoter : la marque de Castor.]\nCastor" : "[A diamond is drawn, then gnawed: the Castor mark.]\nCastor"],
    [start("cascade"), start("cascade", 70), `${t.tagline1}\n${t.tagline2}`],
    [start("cascade", 70), end("cascade"), `${t.toolsLine}.\n${fr ? "PDF 28 · Image 25 · Vidéo 19 · Audio 9 · Créer 20 · Fichiers 9" : "PDF 28 · Image 25 · Video 19 · Audio 9 · Create 20 · Files 9"}`],
    [start("compress"), start("compress", 112), t.figDrop],
    [start("compress", 112), start("compress", 236), t.figCompress],
    [start("compress", 236), end("compress"), `${t.bigBefore} → ${t.bigAfter} (${t.bigPct})\n${t.bigNote}`],
    [start("rmbg"), end("rmbg"), t.figRmbg],
    [start("studio"), end("studio"), t.figStudio],
    [
      start("palette"),
      end("palette"),
      fr ? `⌘K ou Ctrl K. La palette comprend : ${quote(lang, t.query)} trouve Compresser un PDF.` : `⌘K or Ctrl K. ${t.figPalette} ${quote(lang, t.query)} finds Compress PDF.`,
    ],
    [start("panel"), end("panel"), t.figPanel],
    [start("chain"), end("chain"), t.figChain],
    [start("runs"), end("runs"), `${t.figRuns}\n${t.undo}`],
    [start("cli"), start("cli", 96), `$ ${t.cliCmd}\n${t.cliOut[0].trim()}`],
    [start("cli", 96), end("cli"), `$ ${t.mcpCmd}\n${t.figCli}`],
    [start("numbers"), end("numbers"), `4. ${t.sec4}\n${t.rows.map(([k, v]) => `${k} : ${v}`).join(" · ").replace(/ : /g, fr ? " : " : ": ")}`],
    [start("end"), TL.end[0] + TL.end[1], `${t.url}\n${t.joke}`],
  ];
  return "WEBVTT\n\n" + cues.map(([a, b, text], i) => `${i + 1}\n${ts(a)} --> ${ts(b)}\n${text}\n`).join("\n");
};
