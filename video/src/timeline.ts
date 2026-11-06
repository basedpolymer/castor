// The timeline of the promo, in frames at 30 fps.
export const FPS = 30;
// [from, duration] in frames; neighbours overlap by 12 frames (cross-fades)
export const TL = {
  intro: [0, 172],
  cascade: [160, 300],
  compress: [448, 312],
  rmbg: [748, 140],
  studio: [876, 150],
  palette: [1014, 118],
  panel: [1120, 118],
  chain: [1226, 128],
  runs: [1342, 128],
  cli: [1458, 180],
  numbers: [1626, 150],
  end: [1764, 366],
} as const;
export const TOTAL = TL.end[0] + TL.end[1]; // 2130 frames = 71 s

