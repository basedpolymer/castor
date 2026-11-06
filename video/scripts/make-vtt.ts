// Writes out/castor-promo-{en,fr}.vtt (bundled by esbuild in render.mjs, then run by node).
import fs from "node:fs";
import path from "node:path";
import { vtt } from "../src/captions";

const out = process.argv[2];
for (const lang of ["en", "fr"] as const) {
  fs.writeFileSync(path.join(out, `castor-promo-${lang}.vtt`), vtt(lang), "utf8");
}
