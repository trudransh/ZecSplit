// The hero art: the real Zcash and Bitcoin logos rasterised into monospace glyphs, spinning,
// with ciphertext flowing from the ZEC coin to the BTC coin over a dot-matrix field.
// Cell grid, supersampled coverage and the wake field follow stackedlol/ascii-sharks.

// Simple Icons (CC0), 24×24 viewBox: a disc with the Z / ₿ cut out of it.
export const ZEC_PATH =
  "M12 0A12 12 0 0 0 0 12a12.013 12.013 0 0 0 12 12 12 12 0 1 0 0-24zm-1.008 4.418h2.014v2.014l3.275-.002v1.826l-5.08 6.889h5.08v2.423h-3.275v2.006h-2.012v-2.006H7.72v-1.826l5.074-6.888H7.719V6.432h3.273z";
export const BTC_PATH =
  "M23.638 14.904c-1.602 6.43-8.113 10.34-14.542 8.736C2.67 22.05-1.244 15.525.362 9.105 1.962 2.67 8.475-1.243 14.9.358c6.43 1.605 10.342 8.115 8.738 14.548v-.002zm-6.35-4.613c.24-1.59-.974-2.45-2.64-3.03l.54-2.153-1.315-.33-.525 2.107c-.345-.087-.705-.167-1.064-.25l.526-2.127-1.32-.33-.54 2.165c-.285-.067-.565-.132-.84-.2l-1.815-.45-.35 1.407s.975.225.955.236c.535.136.63.486.615.766l-1.477 5.92c-.075.166-.24.406-.614.314.015.02-.96-.24-.96-.24l-.66 1.51 1.71.426.93.242-.54 2.19 1.32.327.54-2.17c.36.1.705.19 1.05.273l-.51 2.154 1.32.33.545-2.19c2.24.427 3.93.257 4.64-1.774.57-1.637-.03-2.58-1.217-3.196.854-.193 1.5-.76 1.68-1.93h.01zm-3.01 4.22c-.404 1.64-3.157.75-4.05.53l.72-2.9c.896.23 3.757.67 3.33 2.37zm.41-4.24c-.37 1.49-2.662.735-3.405.55l.654-2.64c.744.18 3.137.524 2.75 2.084v.006z";

const RAMP = "%#8&@";
const CIPHER = "#%&@$*?!/\\<>=+~^";
const SAMPLES = 4;

type RGB = [number, number, number];
type Kind = "zec" | "btc";

interface Coin {
  kind: Kind;
  path: Path2D;
  /** Centre and radius as fractions of the canvas, so the HTML labels can line up. */
  fx: number;
  fy: number;
  fr: number;
  phase: number;
}

function hash(a: number, b: number, c: number) {
  let h = (a * 374761393 + b * 668265263 + c * 2147483647) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const rgba = ([r, g, b]: RGB, a: number) => `rgba(${r},${g},${b},${clamp(a, 0, 1).toFixed(3)})`;

function readRgb(s: CSSStyleDeclaration, name: string): RGB {
  const p = s.getPropertyValue(name).trim().split(/\s+/).map(Number);
  return [p[0] || 0, p[1] || 0, p[2] || 0];
}

/** Coin layout shared with the page: centres and radius in fractions of the art box. */
export const COINS = {
  zec: { fx: 0.31, fy: 0.33, fr: 0.22 },
  btc: { fx: 0.7, fy: 0.7, fr: 0.2 },
};

export class AsciiCoins {
  private ctx: CanvasRenderingContext2D;
  private sprite = document.createElement("canvas");
  private sg: CanvasRenderingContext2D;
  private coins: Coin[];
  private W = 0;
  private H = 0;
  private dpr = 1;
  private cw = 10;
  private ch = 15;
  private cols = 0;
  private rows = 0;
  private font = "monospace";
  private ink: RGB = [32, 22, 19];
  private dots: RGB = [32, 22, 19];
  private zec: RGB = [244, 183, 40];
  private btc: RGB = [247, 147, 26];
  private energy = new Float32Array(0);
  private tmp = new Float32Array(0);
  private pointer = { x: 0, y: 0, on: false };
  private time = 0;
  private last = 0;
  private raf = 0;
  private visible = true;
  private disposed = false;
  private motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  private resizeObs: ResizeObserver;
  private visObs: IntersectionObserver;
  private themeObs: MutationObserver;

  constructor(private canvas: HTMLCanvasElement) {
    const ctx = canvas.getContext("2d");
    const sg = this.sprite.getContext("2d", { willReadFrequently: true });
    if (!ctx || !sg) throw new Error("2D canvas unavailable");
    this.ctx = ctx;
    this.sg = sg;
    this.coins = [
      { kind: "zec", path: new Path2D(ZEC_PATH), ...COINS.zec, phase: 0 },
      { kind: "btc", path: new Path2D(BTC_PATH), ...COINS.btc, phase: 2.1 },
    ];
    this.readTheme();
    this.resizeObs = new ResizeObserver(() => this.resize());
    this.resizeObs.observe(canvas);
    this.visObs = new IntersectionObserver(([e]) => {
      this.visible = e.isIntersecting;
      this.schedule();
    });
    this.visObs.observe(canvas);
    this.themeObs = new MutationObserver(() => {
      this.readTheme();
      this.draw(0);
    });
    this.themeObs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    this.motion.addEventListener("change", this.onMotion);
    window.addEventListener("pointermove", this.onMove, { passive: true });
    window.addEventListener("pointerdown", this.onDown, { passive: true });
    this.resize();
    // Canvas text doesn't trigger a webfont load, so redraw once the mono face arrives.
    document.fonts?.load(`500 14px ${this.font}`).then(() => !this.disposed && this.draw(0));
    this.schedule();
  }

  destroy() {
    this.disposed = true;
    cancelAnimationFrame(this.raf);
    this.resizeObs.disconnect();
    this.visObs.disconnect();
    this.themeObs.disconnect();
    this.motion.removeEventListener("change", this.onMotion);
    window.removeEventListener("pointermove", this.onMove);
    window.removeEventListener("pointerdown", this.onDown);
  }

  private readTheme() {
    const s = getComputedStyle(this.canvas);
    this.font = s.fontFamily || "monospace";
    this.ink = readRgb(s, "--ink-rgb");
    this.dots = readRgb(s, "--dot-rgb");
    this.zec = readRgb(s, "--zec-rgb");
    this.btc = readRgb(s, "--btc-rgb");
  }

  private resize() {
    const r = this.canvas.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return;
    this.W = r.width;
    this.H = r.height;
    this.dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.cw = clamp(Math.round(r.width / 86), 5, 9);
    this.ch = Math.round(this.cw * 1.5);
    this.cols = Math.ceil(r.width / this.cw);
    this.rows = Math.ceil(r.height / this.ch);
    this.canvas.width = Math.round(r.width * this.dpr);
    this.canvas.height = Math.round(r.height * this.dpr);
    this.energy = new Float32Array(this.cols * this.rows);
    this.tmp = new Float32Array(this.cols * this.rows);
    this.draw(0);
  }

  private schedule() {
    cancelAnimationFrame(this.raf);
    if (this.disposed || !this.visible || this.motion.matches) return;
    this.last = performance.now();
    this.raf = requestAnimationFrame(this.frame);
  }

  private frame = (now: number) => {
    const dt = Math.min(0.05, (now - this.last) / 1000);
    this.last = now;
    this.draw(dt);
    this.raf = requestAnimationFrame(this.frame);
  };

  private onMotion = () => (this.motion.matches ? this.draw(0) : this.schedule());

  private local(e: PointerEvent) {
    const r = this.canvas.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    return { x, y, inside: x >= 0 && y >= 0 && x <= r.width && y <= r.height };
  }

  private onMove = (e: PointerEvent) => {
    const { x, y, inside } = this.local(e);
    this.pointer = { x, y, on: inside && e.pointerType !== "touch" };
  };

  private onDown = (e: PointerEvent) => {
    const { x, y, inside } = this.local(e);
    if (inside) this.stamp(x, y, 5, 1.4);
  };

  private stamp(x: number, y: number, radius: number, amount: number) {
    const c0 = Math.floor(x / this.cw);
    const r0 = Math.floor(y / this.ch);
    for (let r = r0 - radius; r <= r0 + radius; r++) {
      for (let c = c0 - radius; c <= c0 + radius; c++) {
        if (c < 0 || r < 0 || c >= this.cols || r >= this.rows) continue;
        const d = Math.hypot(c - c0, (r - r0) * 1.5);
        if (d > radius + 0.5) continue;
        const i = r * this.cols + c;
        this.energy[i] = Math.min(1.5, this.energy[i] + amount * (1 - d / (radius + 1)));
      }
    }
  }

  private diffuse(dt: number) {
    const { cols, rows, energy: e, tmp } = this;
    const decay = Math.exp(-dt * 1.4);
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c;
        const l = c > 0 ? e[i - 1] : e[i];
        const rt = c < cols - 1 ? e[i + 1] : e[i];
        const u = r > 0 ? e[i - cols] : e[i];
        const d = r < rows - 1 ? e[i + cols] : e[i];
        tmp[i] = (e[i] * 4 + l + rt + u + d) * 0.125 * decay;
      }
    }
    this.energy = tmp;
    this.tmp = e;
  }

  private place(c: Coin) {
    const s = Math.min(this.W, this.H);
    return { x: c.fx * this.W, y: c.fy * this.H, r: c.fr * s };
  }

  /** A point on the arc from the ZEC coin to the BTC coin. */
  private arc(t: number, lane: number) {
    const a = this.place(this.coins[0]);
    const b = this.place(this.coins[1]);
    const ax = a.x + a.r * 0.75;
    const ay = a.y + a.r * 0.75;
    const bx = b.x - b.r * 0.8;
    const by = b.y - b.r * 0.7;
    const mx = (ax + bx) / 2 + (by - ay) * 0.45;
    const my = (ay + by) / 2 - (bx - ax) * 0.25;
    const u = 1 - t;
    const sway = Math.sin(Math.PI * t) * lane * this.ch * 1.6;
    return { x: u * u * ax + 2 * u * t * mx + t * t * bx + sway, y: u * u * ay + 2 * u * t * my + t * t * by - sway * 0.5 };
  }

  private draw(dt: number) {
    const { ctx, dpr, W, H, cw, ch, cols, rows } = this;
    if (!cols) return;
    this.time += dt;
    const t = this.motion.matches ? 1.1 : this.time;

    const notes = 22;
    const plain: { c: number; r: number; g: string; a: number; btc: boolean }[] = [];
    for (let n = 0; n < notes; n++) {
      const p = (n / notes + t * (0.045 + hash(n, 1, 3) * 0.03)) % 1;
      const { x, y } = this.arc(p, hash(n, 2, 5) * 2 - 1);
      const c = Math.floor(x / cw);
      const r = Math.floor(y / ch);
      if (c < 0 || r < 0 || c >= cols || r >= rows) continue;
      if (dt) this.energy[r * cols + c] = Math.min(1.5, this.energy[r * cols + c] + 0.05);
      const toBtc = p > 0.86;
      plain.push({
        c,
        r,
        g: toBtc ? "-" : CIPHER[Math.floor(hash(n, Math.floor(t * 12), 9) * CIPHER.length)],
        a: Math.sin(Math.PI * p) * 0.8 + 0.2,
        btc: toBtc,
      });
    }
    if (this.pointer.on && dt) this.stamp(this.pointer.x, this.pointer.y, 1, 0.14);
    if (dt) this.diffuse(dt);

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);

    // Dot matrix: every other cell, brighter where the wake passes.
    const dot = Math.max(1.5, cw * 0.18);
    for (let r = 0; r < rows; r += 1) {
      for (let c = (r & 1); c < cols; c += 2) {
        const e = this.energy[r * cols + c];
        const a = 0.13 + Math.min(1, e) * 0.75;
        ctx.fillStyle = e > 0.25 ? rgba(this.zec, a) : rgba(this.dots, a);
        const s = e > 0.25 ? dot * 1.6 : dot;
        ctx.fillRect(c * cw + cw / 2 - s / 2, r * ch + ch / 2 - s / 2, s, s);
      }
    }

    ctx.font = `500 ${Math.round(ch * 0.78)}px ${this.font}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (const n of plain) {
      ctx.clearRect(n.c * cw, n.r * ch, cw, ch);
      ctx.fillStyle = n.btc ? rgba(this.btc, n.a) : rgba(this.ink, n.a * 0.75);
      ctx.fillText(n.g, n.c * cw + cw / 2, n.r * ch + ch / 2 + 1);
    }
    for (const coin of this.coins) this.drawCoin(coin, t);
  }

  private drawCoin(coin: Coin, t: number) {
    const { ctx, sg, sprite, cw, ch, cols, rows } = this;
    const { x, y, r } = this.place(coin);
    const angle = t * 0.5 + coin.phase;
    const k = Math.max(Math.abs(Math.cos(angle)), 0.06);
    const rim = r * 0.09 * Math.sin(angle);
    const c0 = Math.max(0, Math.floor((x - r - Math.abs(rim)) / cw) - 1);
    const c1 = Math.min(cols - 1, Math.ceil((x + r + Math.abs(rim)) / cw) + 1);
    const r0 = Math.max(0, Math.floor((y - r) / ch) - 1);
    const r1 = Math.min(rows - 1, Math.ceil((y + r) / ch) + 1);
    const bw = c1 - c0 + 1;
    const bh = r1 - r0 + 1;
    if (bw < 2 || bh < 2) return;

    // Blue = the coin's edge as it turns, green = the whole disc, red = the logo face.
    // Disc without face is the letter, so Z and ₿ get their own colour like the real marks.
    sprite.width = bw * SAMPLES;
    sprite.height = bh * SAMPLES;
    sg.setTransform(SAMPLES / cw, 0, 0, SAMPLES / ch, -c0 * SAMPLES, -r0 * SAMPLES);
    sg.fillStyle = "rgb(0,0,255)";
    sg.beginPath();
    sg.ellipse(x + rim, y, r * k, r, 0, 0, Math.PI * 2);
    sg.fill();
    sg.fillRect(x + Math.min(0, rim), y - r, Math.abs(rim), r * 2);
    sg.save();
    sg.translate(x, y);
    sg.scale((k * r) / 12, r / 12);
    sg.translate(-12, -12);
    sg.fillStyle = "rgb(0,255,0)";
    sg.beginPath();
    sg.ellipse(12, 12, 12, 12, 0, 0, Math.PI * 2);
    sg.fill();
    sg.fillStyle = "rgb(255,255,0)";
    sg.fill(coin.path);
    sg.restore();

    const img = sg.getImageData(0, 0, sprite.width, sprite.height).data;
    const tint = coin.kind === "zec" ? this.zec : this.btc;
    const letter = coin.kind === "zec" ? "#1d1a17" : "#ffffff";
    const letterGlyph = coin.kind === "zec" ? rgba(tint, 0.55) : rgba(tint, 0.45);
    const n = SAMPLES * SAMPLES;
    const sweep = Math.sin(angle * 2) * 0.5;
    for (let rr = 0; rr < bh; rr++) {
      for (let cc = 0; cc < bw; cc++) {
        let face = 0;
        let hole = 0;
        let edge = 0;
        for (let sy = 0; sy < SAMPLES; sy++) {
          for (let sx = 0; sx < SAMPLES; sx++) {
            const o = ((rr * SAMPLES + sy) * sprite.width + cc * SAMPLES + sx) * 4;
            if (img[o + 3] < 100) continue;
            if (img[o] > 128) face++;
            else if (img[o + 1] > 128) hole++;
            else edge++;
          }
        }
        if ((face + hole + edge) / n < 0.5) continue;
        const gc = c0 + cc;
        const gr = r0 + rr;
        const px = gc * cw;
        const py = gr * ch;
        const key = hash(gc, gr, coin.kind === "zec" ? 3 : 7);
        const glyph = RAMP[Math.floor(key * RAMP.length)];
        ctx.clearRect(px, py, cw, ch);
        if (edge > face && edge > hole) {
          ctx.fillStyle = rgba(tint, 0.55);
          ctx.fillRect(px + 0.5, py + 0.5, cw - 1, ch - 1);
          ctx.fillStyle = rgba(this.ink, 0.35);
          ctx.fillText("|", px + cw / 2, py + ch / 2 + 1);
          continue;
        }
        if (hole > face) {
          ctx.fillStyle = letter;
          ctx.fillRect(px + 0.5, py + 0.5, cw - 1, ch - 1);
          ctx.fillStyle = letterGlyph;
          ctx.fillText(glyph, px + cw / 2, py + ch / 2 + 1);
          continue;
        }
        // Lit from the top left, with a highlight that sweeps across as the coin turns.
        const nx = (px + cw / 2 - x) / (r * k);
        const ny = (py + ch / 2 - y) / r;
        const lit = clamp(0.6 - nx * 0.2 - ny * 0.25 + Math.exp(-((nx - sweep) ** 2) * 6) * 0.3 + (key - 0.5) * 0.08, 0, 1);
        ctx.fillStyle = rgba(tint, 0.8 + lit * 0.2);
        ctx.fillRect(px + 0.5, py + 0.5, cw - 1, ch - 1);
        ctx.fillStyle = rgba(this.ink, 0.12 + (1 - lit) * 0.22);
        ctx.fillText(glyph, px + cw / 2, py + ch / 2 + 1);
      }
    }
  }
}
