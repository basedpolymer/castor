// Where things are in the screenshots (pixels of video/public/shots/<lang>/*.png, 1400 px wide),
// measured on the real shots. The French UI is longer, so a few positions differ.
import type { Rect } from "../components/Screen";
import type { Lang } from "../i18n";

export const SHOT = { w: 1400, h: 875 };
export const RMBG = { w: 1400, h: 1080 };
export const PANEL = { w: 360, h: 520 };

type G = {
  dropZone: Rect;
  suggestedCompress: Rect;
  compressButton: Rect;
  statusCells: Rect[]; // "Ready" cells that get the progress bars
  rowBg: string[];
  doneLine: Rect;
  doneJoke: Rect;
  paletteText: Rect; // the typed query in the palette field
  paletteRow: Rect;
  fieldBg: string;
  chainSteps: Rect[];
  renameRow: Rect;
  studioCuts: Rect[]; // the two cut-out parts on the studio timeline
  clipboard: Rect; // in the panel shot
  clipboardFirst: Rect; // "Remove background" chip in the panel
};

const common = {
  dropZone: [260, 158, 1096, 79] as Rect,
  suggestedCompress: [260, 354, 210, 128] as Rect,
  statusCells: [
    [742, 252, 182, 24],
    [742, 296, 182, 24],
  ] as Rect[],
  rowBg: ["#f4f2ee", "#fcfbf9"],
  paletteRow: [397, 149, 606, 34] as Rect,
  fieldBg: "#fcfbf9",
  renameRow: [260, 369, 1096, 26] as Rect,
  clipboard: [10, 122, 342, 48] as Rect,
  clipboardFirst: [13, 149, 97, 18] as Rect,
};

export const GEO: Record<Lang, G> = {
  en: {
    ...common,
    compressButton: [1016, 527, 336, 28],
    doneLine: [258, 368, 320, 24],
    doneJoke: [584, 368, 210, 24],
    paletteText: [411, 80, 114, 20],
    studioCuts: [
      [449, 692, 62, 112],
      [829, 692, 44, 112],
    ],
    chainSteps: [
      [586, 197, 162, 22],
      [775, 197, 128, 22],
      [931, 197, 160, 22],
    ],
  },
  fr: {
    ...common,
    compressButton: [1016, 543, 336, 28],
    doneLine: [258, 368, 352, 24],
    doneJoke: [617, 368, 250, 24],
    paletteText: [411, 80, 124, 20],
    studioCuts: [
      [317, 692, 136, 112],
      [1173, 692, 58, 112],
    ],
    chainSteps: [
      [586, 197, 172, 22],
      [785, 197, 170, 22],
      [982, 197, 200, 22],
    ],
    clipboard: [10, 122, 342, 66],
    clipboardFirst: [13, 149, 111, 18],
  },
};
