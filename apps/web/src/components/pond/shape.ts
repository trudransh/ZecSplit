import { smoothstep } from "./world";

export const FLAG_BODY = 1;
export const FLAG_FIN = 2;
export const FLAG_EYE = 3;
export const FLAG_DORSAL = 4;
export const FLAG_BARBEL = 5;

/** Where a point landed on the koi; `du`/`da` give the fin-ray direction in body space. */
export interface ShapeHit {
  flag: number;
  across: number;
  du: number;
  da: number;
}

/** Top-down outline as a fraction of length: blunt round head, plump shoulders, narrow tail stalk. */
export function bodyHalfWidth(s: number) {
  if (s < 0.12) return 0.11 * Math.pow(Math.sin(((s / 0.12) * Math.PI) / 2), 0.55);
  if (s < 0.3) return 0.11 + 0.025 * smoothstep(0.12, 0.3, s);
  return 0.033 + 0.102 * (1 - smoothstep(0.3, 0.74, s));
}

const BODY_END = 0.74;

/** Butterfly tail: fans out from the stalk, with a V notch in the trailing edge. */
function tailHalfWidth(s: number) {
  return 0.03 + 0.15 * Math.pow(smoothstep(0.7, 1, s), 0.8);
}

/** Test a point against a fin: an ellipse leaning back from a root on the body's edge. */
function inFin(s: number, a: number, rootS: number, angle: number, reach: number, width: number, out: ShapeHit) {
  const rootA = bodyHalfWidth(rootS) * 0.85;
  const cosA = Math.cos(angle);
  const sinA = Math.sin(angle);
  const du = s - rootS;
  const da = a - rootA;
  const along = du * cosA + da * sinA - reach;
  const perp = -du * sinA + da * cosA;
  if ((along / reach) ** 2 + (perp / width) ** 2 > 1) return false;
  out.flag = FLAG_FIN;
  out.du = du;
  out.da = da;
  return true;
}

/**
 * Classify a point given in body coordinates, all as fractions of length:
 * `s` along the spine (0 snout, 1 tail tip), `a` distance from it, and
 * `past` how far it sits beyond the tail tip. `flare` spreads the fins.
 */
export function classify(s: number, a: number, past: number, flare: number, out: ShapeHit, ahead = 0, lateral = 0) {
  out.flag = 0;
  out.across = 1;
  out.du = 1;
  out.da = 0;

  if (ahead > 0) {
    if (ahead < 0.05 && Math.abs(lateral - (0.045 + ahead * 0.7)) < 0.016) {
      out.flag = FLAG_BARBEL;
      return true;
    }
    return false;
  }

  const w = bodyHalfWidth(s);
  if (s <= BODY_END && a <= w) {
    const across = a / w;
    out.across = across;
    if (s > 0.055 && s < 0.1 && across > 0.5 && across < 0.85) out.flag = FLAG_EYE;
    else if (s > 0.28 && s < 0.6 && across < 0.14) out.flag = FLAG_DORSAL;
    else out.flag = FLAG_BODY;
    return true;
  }

  if (s > 0.68 && past <= 0) {
    const tw = tailHalfWidth(s) * flare;
    if (a <= tw && s <= 0.86 + 0.14 * (a / tw)) {
      out.flag = FLAG_FIN;
      out.da = (a / tw) * 0.8;
      return true;
    }
  }

  return (
    inFin(s, a, 0.19, 0.85 + 0.5 * (flare - 1), 0.066 * flare, 0.04, out) ||
    inFin(s, a, 0.47, 0.75, 0.036, 0.018, out)
  );
}


export const FLAG_GILL = 6;
export const FLAG_MARK = 7;

export type SharkKind = "greathammer" | "mako" | "tiger" | "blacktip";

export interface SharkShape {
  halfWidth: (s: number) => number;
  /** Pectoral fin: leading root, trailing root, tip along the body, reach. */
  pectoral: [number, number, number, number];
  /** Sickle fins get a concave trailing edge. */
  falcate: boolean;
  pelvic: [number, number, number, number];
  caudal: number;
  dorsal: [number, number];
  secondDorsal: [number, number];
  /** Hammerhead's cephalofoil: half-span, sweep of its tips, depth, notch at the centre. */
  head?: { span: number; sweep: number; depth: number; notch: number };
  eye: [number, number] | null;
  gills: [number, number];
  /** Tiger bars or blacktip fin tips. */
  marks?: "bars" | "tips";
}

function sharkHalfWidth(s: number) {
  if (s < 0.13) return 0.09 * Math.pow(Math.sin(((s / 0.13) * Math.PI) / 2), 0.95);
  if (s < 0.3) return 0.09 + 0.016 * smoothstep(0.13, 0.3, s);
  if (s < 0.84) return 0.019 + 0.087 * Math.pow(1 - (s - 0.3) / 0.54, 1.25);
  const k = (s - 0.84) / 0.16;
  return 0.019 * (1 - k) + 0.012 * Math.sin(k * Math.PI);
}

export const SHARKS: Record<SharkKind, SharkShape> = {
  greathammer: {
    halfWidth: (s) => {
      if (s < 0.05) return 0.04;
      if (s < 0.3) return 0.04 + 0.07 * smoothstep(0.05, 0.3, s);
      return sharkHalfWidth(s) * 0.95;
    },
    head: { span: 0.21, sweep: 0.006, depth: 0.085, notch: 0.014 },
    pectoral: [0.16, 0.38, 0.49, 0.24],
    falcate: true,
    pelvic: [0.55, 0.62, 0.67, 0.085],
    caudal: 0.13,
    dorsal: [0.24, 0.45],
    secondDorsal: [0.68, 0.74],
    eye: null,
    gills: [0.15, 0.25],
  },
  mako: {
    halfWidth: (s) => {
      if (s < 0.17) return 0.088 * Math.pow(Math.sin(((s / 0.17) * Math.PI) / 2), 1.5);
      if (s < 0.32) return 0.088 + 0.012 * smoothstep(0.17, 0.32, s);
      if (s < 0.86) return 0.014 + 0.086 * Math.pow(1 - (s - 0.32) / 0.54, 1.45);
      const k = (s - 0.86) / 0.14;
      return 0.014 * (1 - k) + 0.01 * Math.sin(k * Math.PI);
    },
    pectoral: [0.18, 0.4, 0.5, 0.21],
    falcate: true,
    pelvic: [0.58, 0.64, 0.68, 0.06],
    caudal: 0.13,
    dorsal: [0.34, 0.45],
    secondDorsal: [0.76, 0.79],
    eye: [0.11, 0.15],
    gills: [0.2, 0.3],
  },
  tiger: {
    halfWidth: (s) => {
      if (s < 0.13) return 0.1 * Math.pow(Math.sin(((s / 0.13) * Math.PI) / 2), 0.45);
      if (s < 0.3) return 0.1 + 0.014 * smoothstep(0.13, 0.3, s);
      if (s < 0.84) return 0.019 + 0.095 * Math.pow(1 - (s - 0.3) / 0.54, 1.2);
      const k = (s - 0.84) / 0.16;
      return 0.019 * (1 - k) + 0.012 * Math.sin(k * Math.PI);
    },
    pectoral: [0.15, 0.42, 0.53, 0.25],
    falcate: true,
    pelvic: [0.57, 0.65, 0.69, 0.085],
    caudal: 0.095,
    dorsal: [0.33, 0.46],
    secondDorsal: [0.71, 0.75],
    eye: [0.08, 0.12],
    gills: [0.19, 0.28],
    marks: "bars",
  },
  blacktip: {
    halfWidth: sharkHalfWidth,
    pectoral: [0.15, 0.43, 0.55, 0.26],
    falcate: true,
    pelvic: [0.57, 0.65, 0.69, 0.085],
    caudal: 0.085,
    dorsal: [0.33, 0.47],
    secondDorsal: [0.71, 0.76],
    eye: [0.1, 0.14],
    gills: [0.19, 0.29],
    marks: "tips",
  },
};

function inTriangle(px: number, py: number, ax: number, ay: number, bx: number, by: number, cx: number, cy: number) {
  const d1 = (px - bx) * (ay - by) - (ax - bx) * (py - by);
  const d2 = (px - cx) * (by - cy) - (bx - cx) * (py - cy);
  const d3 = (px - ax) * (cy - ay) - (cx - ax) * (py - ay);
  return !((d1 < 0 || d2 < 0 || d3 < 0) && (d1 > 0 || d2 > 0 || d3 > 0));
}

/** Like `classify`, for a shark. `across` on a fin is how far out toward its tip the point sits. */
export function classifyShark(shape: SharkShape, s: number, a: number, flare: number, out: ShapeHit) {
  out.flag = 0;
  out.across = 1;

  const head = shape.head;
  if (head && s < head.sweep + head.depth + head.notch) {
    const r = a / head.span;
    const front = head.sweep * r * r + head.notch * Math.max(0, 1 - r / 0.14);
    const depth = head.depth * (1 - 0.2 * r ** 4);
    if (r <= 1 && s >= front && s <= front + depth) {
      out.flag = r > 0.86 ? FLAG_EYE : FLAG_BODY;
      out.across = r;
      return true;
    }
  }

  const w = shape.halfWidth(s);
  if (a <= w) {
    const across = a / (w || 1);
    out.across = across;
    if (s > shape.dorsal[0] && s < shape.dorsal[1] && across < 0.3) out.flag = FLAG_DORSAL;
    else if (shape.eye && s > shape.eye[0] && s < shape.eye[1] && across > 0.6) out.flag = FLAG_EYE;
    else if (
      s > shape.gills[0] &&
      s < shape.gills[1] &&
      across > 0.58 &&
      Math.floor(((s - shape.gills[0]) / (shape.gills[1] - shape.gills[0])) * 5) % 2 === 0
    )
      out.flag = FLAG_GILL;
    else if (shape.marks === "bars" && s > 0.2 && s < 0.9 && across < 0.85) {
      const bar = Math.sin(s * Math.PI * 18 + 0.35 * Math.sin(a * 25 + s * 5));
      out.flag = bar > 0.65 - 0.5 * smoothstep(0.2, 0.42, s) + 0.4 * smoothstep(0.8, 0.9, s) ? FLAG_MARK : FLAG_BODY;
    } else out.flag = FLAG_BODY;
    return true;
  }

  const tip = shape.pectoral[3] * flare;
  const [p0, p1, p2] = shape.pectoral;
  if (
    inTriangle(s, a, p0, 0, p1, 0, p2, tip) &&
    !(shape.falcate && inTriangle(s, a, p1, 0, p1 + (p2 - p1) * 0.33, tip * 0.55, p2, tip))
  ) {
    out.flag = FLAG_FIN;
    out.across = a / tip;
    return true;
  }
  const [v0, v1, v2, vr] = shape.pelvic;
  if (inTriangle(s, a, v0, 0, v1, 0, v2, vr)) {
    out.flag = FLAG_FIN;
    out.across = a / vr;
    return true;
  }
  if (shape.caudal > 0 && inTriangle(s, a, 0.86, 0, 0.95, 0, 1, shape.caudal)) {
    out.flag = FLAG_FIN;
    out.across = a / shape.caudal;
    return true;
  }
  return false;
}
