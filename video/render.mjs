// Render every deliverable into out/ (npm run render [-- promo-en …] to render only some):
//   castor-promo-{en,fr}.mp4 and -vertical.mp4 (H.264 + AAC, +faststart),
//   castor-promo-{en,fr}.webm (VP9 + Opus, encoded from the MP4 master),
//   castor-promo-{en,fr}[-vertical]-poster.png, castor-promo-{en,fr}.vtt
// Needs ffmpeg on PATH. CHROME=<path> to use another browser than the installed Chrome.
import { bundle } from "@remotion/bundler";
import { renderMedia, renderStill, selectComposition } from "@remotion/renderer";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const root = import.meta.dirname;
const out = path.join(root, "out");
const tmp = path.join(out, "tmp");
fs.mkdirSync(tmp, { recursive: true });
const browserExecutable = process.env.CHROME ?? "C:/Program Files/Google/Chrome/Application/chrome.exe";
const only = process.argv.slice(2);
const POSTER_FRAME = 440; // the tagline over the tool cards

const ffmpeg = (...args) => execFileSync("ffmpeg", ["-v", "error", "-y", ...args], { stdio: "inherit" });

const serveUrl = await bundle({ entryPoint: path.join(root, "src/index.ts"), publicDir: path.join(root, "public") });
for (const lang of ["en", "fr"]) {
  for (const vertical of [false, true]) {
    const id = `promo-${lang}${vertical ? "-vertical" : ""}`;
    if (only.length && !only.includes(id)) continue;
    const name = `castor-promo-${lang}${vertical ? "-vertical" : ""}`;
    const composition = await selectComposition({ serveUrl, id, browserExecutable });
    const master = path.join(tmp, `${name}.mp4`);
    let last = -1;
    await renderMedia({
      composition,
      serveUrl,
      browserExecutable,
      codec: "h264",
      imageFormat: "png", // lossless frames: crisp text
      colorSpace: "bt709", // standard HD colours, limited range, tagged
      crf: 20,
      x264Preset: "slow",
      pixelFormat: "yuv420p",
      audioCodec: "aac",
      audioBitrate: "160k",
      outputLocation: master,
      concurrency: Math.max(2, os.cpus().length - 2),
      onProgress: ({ progress }) => {
        const p = Math.floor(progress * 10);
        if (p !== last) console.log(`${id}: ${p * 10} %`), (last = p);
      },
    });
    // web-friendly: the moov atom first
    ffmpeg("-i", master, "-c", "copy", "-movflags", "+faststart", path.join(out, `${name}.mp4`));
    await renderStill({ composition, serveUrl, browserExecutable, frame: POSTER_FRAME, imageFormat: "png", output: path.join(out, `${name}-poster.png`) });
    if (!vertical) {
      ffmpeg("-i", master, "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "34", "-row-mt", "1", "-deadline", "good", "-cpu-used", "2", "-c:a", "libopus", "-b:a", "128k", path.join(out, `${name}.webm`));
    }
    console.log(`${id}: done`);
  }
}
execFileSync(process.execPath, [path.join(root, "node_modules/esbuild/bin/esbuild"), "scripts/make-vtt.ts", "--bundle", "--platform=node", "--format=esm", `--outfile=${path.join(tmp, "make-vtt.mjs")}`, "--log-level=warning"], { cwd: root, stdio: "inherit" });
execFileSync(process.execPath, [path.join(tmp, "make-vtt.mjs"), out], { stdio: "inherit" });
console.log("done:", fs.readdirSync(out).filter((f) => fs.statSync(path.join(out, f)).isFile()).join(", "));
