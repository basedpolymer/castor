// Render a few frames as PNG to look at while working (one bundle, many stills).
//   node scripts/preview.mjs promo-en 100,300,520 [promo-fr-vertical 900 …]
import { bundle } from "@remotion/bundler";
import { renderStill, selectComposition } from "@remotion/renderer";
import path from "node:path";
import fs from "node:fs";

const browserExecutable = process.env.CHROME ?? "C:/Program Files/Google/Chrome/Application/chrome.exe";
const root = path.resolve(import.meta.dirname, "..");
const serveUrl = await bundle({ entryPoint: path.join(root, "src/index.ts"), publicDir: path.join(root, "public") });
const args = process.argv.slice(2);
fs.mkdirSync(path.join(root, "out/frames"), { recursive: true });
for (let i = 0; i < args.length; i += 2) {
  const id = args[i];
  const composition = await selectComposition({ serveUrl, id, browserExecutable });
  for (const fr of args[i + 1].split(",").map(Number)) {
    const output = path.join(root, `out/frames/${id}-${String(fr).padStart(4, "0")}.png`);
    await renderStill({ composition, serveUrl, output, frame: fr, browserExecutable });
    console.log(output);
  }
}
