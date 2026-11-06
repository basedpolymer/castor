// The bitten diamond (assets/brand/castor-mark.svg), drawn like a TikZ stroke, then inked.
import React from "react";
import { interpolate, Easing } from "remotion";
import { C } from "../theme";

const BEZ = Easing.bezier(0.65, 0, 0.35, 1);
const C_PATH =
  "M522 33 529 63C437 0 369 -26 299 -26C220 -26 170 34 170 129C170 337 317 544 465 544C525 544 555 507 561 428L635 491C626 560 590 591 518 591C431 591 330 540 242 452C145 355 88 222 88 92C88 -11 145 -73 241 -73C327 -73 436 -32 522 33Z";
const SIDE = Math.hypot(300, 300) * 4; // perimeter of the diamond in SVG units

type Props = {
  size: number;
  /** 0 → 1: the outline is drawn */
  draw?: number;
  /** 0 → 1: ink fills the diamond */
  fill?: number;
  /** 0 → 1: the bite is taken (circles grow) */
  bite?: number;
  /** 0 → 1: the calligraphic C appears */
  letter?: number;
  /** 0 → 1: crumbs appear */
  crumbs?: number;
  /** the big crumb falls: 0 = in place; > 0 = seconds since it let go */
  fall?: number;
  /** extra distance (px on screen) the crumb may fall before it rests */
  fallTo?: number;
  id: string;
  color?: string;
  paper?: string;
};

export const Mark: React.FC<Props> = ({
  size,
  draw = 1,
  fill = 1,
  bite = 1,
  letter = 1,
  crumbs = 1,
  fall = 0,
  fallTo = 400,
  id,
  color = C.ink,
  paper = C.paper,
}) => {
  const scale = size / 644; // svg units → px
  // free fall, g ≈ 2600 px/s² on screen, stops at fallTo (no bounce)
  const fallPx = Math.min(fallTo, 0.5 * 2600 * fall * fall);
  const fallU = fallPx / scale;
  const rot = Math.min(1, fall * 1.6) * 160;
  const r = (v: number) => v * bite;
  return (
    <svg width={size} height={size} viewBox="190 190 644 644" style={{ overflow: "visible", display: "block" }}>
      <defs>
        <mask id={`bite-${id}`} maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">
          <rect width="1024" height="1024" fill="#fff" />
          <circle cx="684" cy="384" r={r(40)} fill="#000" />
          <circle cx="730" cy="430" r={r(33)} fill="#000" />
          <circle cx="767" cy="469" r={r(23)} fill="#000" />
        </mask>
      </defs>
      <g mask={`url(#bite-${id})`}>
        <polygon points="512,212 812,512 512,812 212,512" fill={color} opacity={fill} />
        <polygon
          points="512,212 812,512 512,812 212,512"
          fill="none"
          stroke={color}
          strokeWidth={Math.min(14, 3 / scale)}
          strokeLinejoin="miter"
          strokeDasharray={SIDE}
          strokeDashoffset={SIDE * (1 - draw)}
          opacity={1 - fill * 0.9}
        />
      </g>
      <g fill={color} opacity={crumbs}>
        <rect
          x="782"
          y="356"
          width="16"
          height="9"
          transform={`translate(0 ${fallU}) rotate(${-28 + rot} 790 360)`}
        />
        <rect x="812" y="398" width="11" height="7" transform="rotate(22 817 401)" />
        <rect x="752" y="326" width="9" height="6" transform="rotate(50 756 329)" />
      </g>
      <g transform="translate(335 646) scale(0.47 -0.47)" fill={paper} opacity={letter}>
        <path d={C_PATH} />
      </g>
    </svg>
  );
};

/** progress helper: 0 → 1 between two frames, eased */
export const prog = (frame: number, a: number, b: number, ease = BEZ) =>
  interpolate(frame, [a, b], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: ease });
