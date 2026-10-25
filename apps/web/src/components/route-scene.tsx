"use client";

import { useEffect, useRef, useState } from "react";
import { BTC_PATH, ZEC_PATH } from "./ascii-coins";

// Each scene is a small three.js story in steps, with a caption saying what just happened.
type Name = "bridge" | "exchange" | "pool" | "safety" | "notes" | "split" | "adaptor";
type Three = typeof import("three");

const STORY: Record<Name, string[]> = {
  bridge: [
    "Your ZEC sits in your shielded wallet. Nobody can see it.",
    "A bridge needs it on a public t1 address first. Now everyone can see the amount.",
    "The bridge's vault takes custody of your coins.",
    "The operator can hold them there: frozen for review.",
  ],
  exchange: [
    "You deposit ZEC to an address the exchange controls.",
    "It demands your passport (KYC) and keeps a copy.",
    "You ask to withdraw your coins…",
    "…and the exchange can freeze your account instead.",
  ],
  pool: [
    "Your ZEC goes into the pool's vault: a public address anyone can watch.",
    "The pool sends BTC out to your Bitcoin address.",
    "Anyone watching the vault follows the coins in and out…",
    "…and links your ZEC to your BTC. Forever.",
  ],
  safety: [
    "Maker never locks BTC: you never pay, so there's nothing to undo.",
    "You never pay: at deadline T1 the swap cancels and the maker's BTC goes back to them.",
    "Maker goes silent after you pay: at T1 the app cancels, the maker's refund reveals their half of the key, and your ZEC comes home.",
    "Maker never refunds: at deadline T2 the app claims the maker's BTC as a penalty.",
  ],
  notes: [
    "A payment is sealed into an encrypted note.",
    "Anyone watching sees only ciphertext.",
    "A zero-knowledge proof shows it's valid without opening it.",
  ],
  split: [
    "Two secret halves: one is yours, one is the maker's.",
    "Your half alone opens nothing.",
    "Only both halves together can spend the ZEC.",
  ],
  adaptor: [
    "You claim the BTC with your signature…",
    "…and that signature reveals your half of the key.",
    "The maker completes the key and sweeps the ZEC. One action settles both sides.",
  ],
};

/** Step length and the area (in scene units) the camera keeps in frame. */
const CONFIG: Record<Name, { ms: number; w: number; h: number; lookY: number }> = {
  bridge: { ms: 3000, w: 5.2, h: 3.2, lookY: -0.05 },
  exchange: { ms: 3000, w: 5.2, h: 3.2, lookY: -0.05 },
  pool: { ms: 3000, w: 5.2, h: 3.2, lookY: -0.05 },
  safety: { ms: 7500, w: 5.6, h: 4.2, lookY: 0.45 },
  notes: { ms: 3200, w: 3.9, h: 3.0, lookY: 0.2 },
  split: { ms: 3200, w: 3.9, h: 3.0, lookY: 0.15 },
  adaptor: { ms: 3200, w: 3.9, h: 3.0, lookY: 0.2 },
};

const GREEN = "#5cb86b";

const GOLD = "#f4c55e";
const LEAK = "#f08a4b";
const INK = "#cfc8bd";

const clamp = (v: number, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, v));
const ease = (u: number) => (u < 0.5 ? 2 * u * u : 1 - (-2 * u + 2) ** 2 / 2);
/** Motion fills the first 60% of a step; the rest holds so it can be read. */
const move = (u: number) => ease(clamp(u / 0.6));
/** Eased progress of `u` across the window [a, b]. */
const seg = (u: number, a: number, b: number) => ease(clamp((u - a) / (b - a)));

function canvasTexture(T: Three, w: number, h: number, draw: (g: CanvasRenderingContext2D) => void) {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const g = c.getContext("2d");
  if (g) draw(g);
  const tex = new T.CanvasTexture(c);
  tex.colorSpace = T.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

/** Shared building blocks: coins, boxes, labels, watchers and a cage. */
function kit(T: Three, scene: import("three").Scene) {
  const add = <O extends import("three").Object3D>(o: O) => (scene.add(o), o);

  const label = (text: string, color = INK, size = 0.19) => {
    const font = `500 64px "JetBrains Mono Variable", ui-monospace, monospace`;
    const probe = document.createElement("canvas").getContext("2d");
    if (probe) probe.font = font;
    const w = Math.ceil((probe?.measureText(text).width ?? text.length * 38) + 24);
    const tex = canvasTexture(T, w, 88, (g) => {
      g.font = font;
      g.fillStyle = color;
      g.textBaseline = "middle";
      g.fillText(text, 12, 46);
    });
    const s = new T.Sprite(new T.SpriteMaterial({ map: tex, transparent: true, depthTest: false }));
    s.scale.set((size * w) / 88, size, 1);
    s.renderOrder = 10;
    return add(s);
  };

  const coin = (kind: "zec" | "btc") => {
    const face = canvasTexture(T, 256, 256, (g) => {
      g.translate(128, 128);
      g.beginPath();
      g.arc(0, 0, 124, 0, Math.PI * 2);
      g.fillStyle = kind === "zec" ? "#1d1a17" : "#ffffff";
      g.fill();
      g.scale(248 / 24, 248 / 24);
      g.translate(-12, -12);
      g.fillStyle = kind === "zec" ? GOLD : "#f7931a";
      g.fill(new Path2D(kind === "zec" ? ZEC_PATH : BTC_PATH));
    });
    const rim = new T.MeshStandardMaterial({ color: kind === "zec" ? "#c9971f" : "#d97a10", metalness: 0.6, roughness: 0.35 });
    const cap = new T.MeshStandardMaterial({ map: face, metalness: 0.35, roughness: 0.4 });
    // Separate faces instead of cylinder caps: a circle's UVs keep the logo upright, and the
    // back face is turned half a revolution so it reads correctly too as the coin spins.
    const edge = new T.CylinderGeometry(0.3, 0.3, 0.07, 48, 1, true);
    edge.rotateX(Math.PI / 2);
    const disc = new T.CircleGeometry(0.3, 48);
    const front = new T.Mesh(disc, cap);
    front.position.z = 0.035;
    const back = new T.Mesh(disc, cap);
    back.position.z = -0.035;
    back.rotation.y = Math.PI;
    const group = new T.Group();
    group.add(new T.Mesh(edge, rim), front, back);
    return add(group);
  };

  const box = (w: number, h: number, d: number, color: string, opacity: number, edge: string) => {
    const g = new T.BoxGeometry(w, h, d);
    const m = new T.Mesh(
      g,
      new T.MeshStandardMaterial({ color, transparent: opacity < 1, opacity, roughness: 0.7, depthWrite: opacity >= 1 }),
    );
    const lines = new T.LineSegments(new T.EdgesGeometry(g), new T.LineBasicMaterial({ color: edge }));
    m.add(lines);
    return add(m);
  };

  /** A row of eyes that pop in and look at whatever is public. */
  const eyes = (anchor: import("three").Vector3) => {
    const group = add(new T.Group());
    const pupils: import("three").Mesh[] = [];
    [-0.42, 0, 0.42].forEach((dx, i) => {
      const white = new T.Mesh(new T.SphereGeometry(0.1, 20, 14), new T.MeshStandardMaterial({ color: "#f3f0ea" }));
      white.position.set(anchor.x + dx, anchor.y + (i === 1 ? 0.12 : 0), anchor.z + 0.4);
      const pupil = new T.Mesh(new T.SphereGeometry(0.05, 14, 10), new T.MeshBasicMaterial({ color: "#111" }));
      white.add(pupil);
      pupils.push(pupil);
      group.add(white);
    });
    const v = new T.Vector3();
    return {
      group,
      update(show: number, target: import("three").Vector3) {
        group.visible = show > 0.01;
        group.children.forEach((w, i) => {
          w.scale.setScalar(Math.max(show, 0.001));
          v.copy(target).sub(w.position).normalize().multiplyScalar(0.065);
          pupils[i].position.copy(v);
        });
      },
    };
  };

  /** Bars that drop around something that's been frozen. */
  const cage = (at: import("three").Vector3, radius: number, height: number) => {
    const group = add(new T.Group());
    const mat = new T.MeshStandardMaterial({ color: LEAK, emissive: LEAK, emissiveIntensity: 0.5 });
    for (let i = 0; i < 12; i++) {
      const a = (i / 12) * Math.PI * 2;
      const bar = new T.Mesh(new T.CylinderGeometry(0.022, 0.022, height, 8), mat);
      bar.position.set(Math.cos(a) * radius, 0, Math.sin(a) * radius);
      group.add(bar);
    }
    const ring = new T.Mesh(new T.TorusGeometry(radius, 0.03, 8, 40), mat);
    ring.rotation.x = Math.PI / 2;
    ring.position.y = height / 2;
    group.add(ring, ring.clone().translateZ(height));
    return {
      group,
      update(drop: number) {
        group.visible = drop > 0;
        group.position.set(at.x, at.y + (1 - drop) * 2.6, at.z);
      },
    };
  };

  /** A soft cone of light: "this is in public view". */
  const beam = (at: import("three").Vector3) => {
    const m = new T.Mesh(
      new T.ConeGeometry(0.62, 1.7, 40, 1, true),
      new T.MeshBasicMaterial({ color: LEAK, transparent: true, opacity: 0, side: T.DoubleSide, depthWrite: false, blending: T.AdditiveBlending }),
    );
    m.position.set(at.x, at.y + 0.85, at.z);
    return add(m);
  };

  const arc = (out: import("three").Vector3, a: import("three").Vector3, b: import("three").Vector3, u: number, lift = 0.9) => {
    out.lerpVectors(a, b, u);
    out.y += Math.sin(Math.PI * u) * lift;
    return out;
  };

  /** A key split into its bow (your half, gold) and blade (the maker's half, orange); same origin when joined. */
  const keyHalves = () => {
    const metal = (c: string) => new T.MeshStandardMaterial({ color: c, metalness: 0.65, roughness: 0.3 });
    const bow = add(new T.Group());
    const ring = new T.Mesh(new T.TorusGeometry(0.15, 0.05, 12, 32), metal(GOLD));
    ring.position.x = -0.32;
    const stem = new T.Mesh(new T.BoxGeometry(0.2, 0.07, 0.07), metal(GOLD));
    stem.position.x = -0.1;
    bow.add(ring, stem);
    const blade = add(new T.Group());
    const shaft = new T.Mesh(new T.BoxGeometry(0.32, 0.07, 0.07), metal(LEAK));
    shaft.position.x = 0.16;
    blade.add(shaft);
    for (const [x, h] of [
      [0.2, 0.12],
      [0.28, 0.08],
    ]) {
      const tooth = new T.Mesh(new T.BoxGeometry(0.05, h, 0.07), metal(LEAK));
      tooth.position.set(x, -h / 2 - 0.02, 0);
      blade.add(tooth);
    }
    return { bow, blade };
  };

  /** A padlock; `open(0..1)` lifts and swings its shackle. */
  const padlock = (edge = GOLD) => {
    const group = add(new T.Group());
    const body = new T.Mesh(
      new T.BoxGeometry(0.7, 0.56, 0.3),
      new T.MeshStandardMaterial({ color: "#2a2622", metalness: 0.4, roughness: 0.5 }),
    );
    body.add(new T.LineSegments(new T.EdgesGeometry(body.geometry), new T.LineBasicMaterial({ color: edge })));
    const hole = new T.Mesh(new T.CircleGeometry(0.06, 20), new T.MeshBasicMaterial({ color: "#0b0a09" }));
    hole.position.z = 0.151;
    body.add(hole);
    const shackle = new T.Mesh(
      new T.TorusGeometry(0.22, 0.045, 12, 32, Math.PI),
      new T.MeshStandardMaterial({ color: "#a7a098", metalness: 0.8, roughness: 0.25 }),
    );
    shackle.position.y = 0.28;
    group.add(body, shackle);
    return {
      group,
      open(v: number) {
        shackle.position.y = 0.28 + v * 0.16;
        shackle.rotation.y = v * 1.2;
      },
    };
  };

  /** A shielded note: a sealed card whose fields are all redacted. */
  const noteCard = () => {
    const tex = canvasTexture(T, 360, 240, (g) => {
      g.fillStyle = "#1d1a17";
      g.fillRect(0, 0, 360, 240);
      g.strokeStyle = GOLD;
      g.lineWidth = 6;
      g.strokeRect(3, 3, 354, 234);
      g.fillStyle = GOLD;
      g.font = `600 30px "JetBrains Mono Variable", monospace`;
      g.fillText("NOTE", 24, 46);
      g.font = `500 22px "JetBrains Mono Variable", monospace`;
      (
        [
          ["to", 200],
          ["amount", 120],
          ["memo", 170],
        ] as const
      ).forEach(([key, w], i) => {
        g.fillStyle = INK;
        g.fillText(key, 24, 100 + i * 46);
        g.fillStyle = GOLD;
        g.fillRect(130, 82 + i * 46, w, 24);
      });
    });
    const edge = new T.MeshStandardMaterial({ color: GOLD });
    return add(
      new T.Mesh(new T.BoxGeometry(1.0, 0.66, 0.03), [
        edge,
        edge,
        edge,
        edge,
        new T.MeshBasicMaterial({ map: tex }),
        new T.MeshStandardMaterial({ color: "#1d1a17" }),
      ]),
    );
  };

  return { label, coin, box, eyes, cage, beam, arc, add, keyHalves, padlock, noteCard };

}

type Kit = ReturnType<typeof kit>;
type Story = (phase: number, u: number, t: number) => void;

function bridge(T: Three, k: Kit): Story {
  const V = (x: number, y: number, z = 0) => new T.Vector3(x, y, z);
  const wallet = k.box(0.95, 0.95, 0.95, "#16130f", 0.82, GOLD);
  wallet.position.set(-1.7, 0, 0);
  const pad = k.box(1.1, 0.08, 1.1, "#2d2823", 1, LEAK);
  pad.position.set(0, -0.42, 0);
  const vault = k.box(1, 1, 1, "#4a443d", 1, "#a7a098");
  vault.position.set(1.7, 0.02, 0);
  k.label("your wallet").position.set(-1.7, -0.78, 0.5);
  k.label("t1 address", LEAK).position.set(0, -0.78, 0.5);
  k.label("bridge vault").position.set(1.7, -0.78, 0.5);
  k.label("shielded", GOLD, 0.15).position.set(-1.7, 0.68, 0);

  const coin = k.coin("zec");
  const amount = k.label("12.50 ZEC · public", LEAK, 0.17);
  const light = k.beam(V(0, -0.38));
  const watchers = k.eyes(V(0, 1.25));
  const cage = k.cage(vault.position, 0.78, 1.15);
  const frozen = k.label("FROZEN", LEAK, 0.24);
  frozen.position.set(1.7, 0.98, 0.4);
  const edge = (vault.children[0] as import("three").LineSegments).material as import("three").LineBasicMaterial;

  const A = V(-1.7, 0), B = V(0, 0.08), C = V(1.7, 0.02, 0.62);
  const p = new T.Vector3();
  return (phase, u, t) => {
    if (phase === 0) p.copy(A);
    else if (phase === 1) k.arc(p, A, B, move(u));
    else if (phase === 2) k.arc(p, B, C, move(u), 0.7);
    else p.copy(C);
    coin.position.copy(p);
    coin.rotation.y = t * 1.6;
    amount.visible = phase === 1 || phase === 2;
    amount.position.set(p.x, p.y + 0.5, p.z + 0.3);
    (light.material as import("three").MeshBasicMaterial).opacity = phase >= 1 ? 0.16 * (phase === 1 ? move(u) : 1) : 0;
    watchers.update(phase === 0 ? 0 : phase === 1 ? move(u) : 1, p);
    cage.update(phase === 3 ? move(u) : 0);
    frozen.visible = phase === 3 && u > 0.45 && Math.floor(t * 3) % 2 === 0;
    edge.color.set(phase === 3 && u > 0.4 ? LEAK : "#a7a098");
  };
}

function exchange(T: Three, k: Kit): Story {
  const V = (x: number, y: number, z = 0) => new T.Vector3(x, y, z);
  const wallet = k.box(0.9, 0.9, 0.9, "#16130f", 0.82, GOLD);
  wallet.position.set(-1.75, 0, 0);
  const ex = k.box(1.05, 1.3, 0.9, "#3a352f", 1, "#a7a098");
  ex.position.set(0, 0.15, 0);
  const out = k.box(0.85, 0.85, 0.85, "#cfc8bd", 0.12, INK);
  out.position.set(1.75, 0, 0);
  k.label("your wallet").position.set(-1.75, -0.78, 0.5);
  k.label("exchange").position.set(0, -0.78, 0.5);
  k.label("withdraw to").position.set(1.75, -0.78, 0.5);

  const card = new T.Mesh(
    new T.PlaneGeometry(0.78, 0.5),
    new T.MeshBasicMaterial({
      transparent: true,
      map: canvasTexture(T, 312, 200, (g) => {
        g.fillStyle = "#ece6da";
        g.beginPath();
        g.roundRect(0, 0, 312, 200, 18);
        g.fill();
        g.fillStyle = "#a59d96";
        g.fillRect(22, 46, 86, 110);
        g.fillStyle = "#2a2622";
        g.font = `600 26px "JetBrains Mono Variable", monospace`;
        g.fillText("PASSPORT", 22, 32);
        for (let i = 0; i < 4; i++) g.fillRect(126, 60 + i * 26, 160 - i * 22, 10);
      }),
    }),
  );
  k.add(card);
  const scan = new T.Mesh(new T.PlaneGeometry(0.86, 0.035), new T.MeshBasicMaterial({ color: LEAK }));
  k.add(scan);
  const stored = k.label("passport · stored", LEAK, 0.15);
  stored.position.set(0, 1.12, 0.5);

  const coin = k.coin("zec");
  const amount = k.label("12.50 ZEC", INK, 0.17);
  const cage = k.cage(V(0, 0.15, 0.15), 0.85, 1.45);
  const frozen = k.label("ACCOUNT FROZEN", LEAK, 0.2);
  frozen.position.set(0, 1.18, 0.5);

  const A = V(-1.75, 0), B = V(0, 0.1, 0.62), C = V(1.75, 0.05, 0.4);
  const cardFrom = V(-1.2, 1.5, 0.7), cardAt = V(0, 0.55, 0.7);
  const p = new T.Vector3();
  return (phase, u, t) => {
    if (phase === 0) k.arc(p, A, B, move(u));
    else if (phase === 1) p.copy(B);
    else if (phase === 2) p.lerpVectors(B, C, ease(clamp(u / 0.7)) * 0.55);
    else p.lerpVectors(B, C, (1 - ease(clamp(u / 0.3))) * 0.55);
    coin.position.copy(p);
    coin.rotation.y = t * 1.6;
    amount.position.set(p.x, p.y + 0.5, p.z + 0.2);
    amount.visible = phase !== 1;

    card.visible = phase === 1;
    scan.visible = phase === 1 && u > 0.35 && u < 0.85;
    if (phase === 1) {
      card.position.lerpVectors(cardFrom, cardAt, move(u * 1.6));
      if (u > 0.85) card.position.z -= (u - 0.85) * 6;
      scan.position.set(card.position.x, card.position.y + 0.22 - clamp((u - 0.35) / 0.5) * 0.44, card.position.z + 0.01);
    }
    stored.visible = phase === 1 && u > 0.8;
    cage.update(phase === 3 ? move(u) : 0);
    frozen.visible = phase === 3 && u > 0.45 && Math.floor(t * 3) % 2 === 0;
  };
}

function pool(T: Three, k: Kit): Story {
  const V = (x: number, y: number, z = 0) => new T.Vector3(x, y, z);
  const wallet = k.box(0.9, 0.9, 0.9, "#16130f", 0.82, GOLD);
  wallet.position.set(-1.8, 0, 0);
  const vault = k.box(1, 1, 1, LEAK, 0.14, LEAK);
  vault.position.set(0, 0, 0);
  const btcWallet = k.box(0.85, 0.85, 0.85, "#16130f", 0.82, "#f7931a");
  btcWallet.position.set(1.8, 0, 0);
  k.label("your ZEC").position.set(-1.8, -0.78, 0.5);
  k.label("public vault", LEAK).position.set(0, -0.78, 0.5);
  k.label("your BTC").position.set(1.8, -0.78, 0.5);

  const zec = k.coin("zec");
  const btc = k.coin("btc");
  const inLabel = k.label("+12.50 ZEC", LEAK, 0.16);
  inLabel.position.set(-0.05, 0.78, 0.4);
  const outLabel = k.label("−0.0461 BTC", LEAK, 0.16);
  outLabel.position.set(0.05, 1.0, 0.4);
  const watchers = k.eyes(V(0, 1.3));

  const curve = new T.CatmullRomCurve3([V(-1.8, 0.05, 0.55), V(-0.9, 0.55, 0.65), V(0, 0.05, 0.6), V(0.9, 0.55, 0.65), V(1.8, 0.05, 0.55)]);
  const tubeGeo = new T.TubeGeometry(curve, 120, 0.03, 8, false);
  const tubeMat = new T.MeshBasicMaterial({ color: LEAK, transparent: true });
  const tube = k.add(new T.Mesh(tubeGeo, tubeMat));
  const total = tubeGeo.index?.count ?? 0;
  const linked = k.label("linked: your ZEC ↔ your BTC", LEAK, 0.17);
  linked.position.set(0, -1.1, 0.6);

  const A = V(-1.8, 0), B = V(0, 0), C = V(1.8, 0.02, 0.55);
  const head = new T.Vector3();
  return (phase, u, t) => {
    zec.position.copy(phase === 0 ? k.arc(new T.Vector3(), A, B, move(u)) : B);
    zec.rotation.y = t * 1.6;
    btc.visible = phase >= 1;
    btc.position.copy(phase === 1 ? k.arc(new T.Vector3(), B, C, move(u)) : C);
    btc.rotation.y = t * 1.6 + 1;
    inLabel.visible = phase === 0 ? u > 0.55 : true;
    outLabel.visible = phase >= 1 && !(phase === 1 && u < 0.2);

    const drawn = phase < 2 ? 0 : phase === 2 ? move(u) : 1;
    tube.visible = drawn > 0;
    tubeGeo.setDrawRange(0, Math.floor((total * drawn) / 6) * 6);
    tubeMat.opacity = phase === 3 ? 0.65 + 0.35 * Math.sin(t * 6) : 1;
    curve.getPointAt(clamp(drawn, 0.001, 1), head);
    watchers.update(phase === 0 ? move(u) : 1, phase >= 2 ? head : phase === 1 ? btc.position : zec.position);
    linked.visible = phase === 3 && u > 0.3;
  };
}

/** All four ways a swap can fail, on Bitcoin's block clock (PROTOCOL_SPEC §5.3). */
function safety(T: Three, k: Kit): Story {
  const V = (x: number, y: number, z = 0) => new T.Vector3(x, y, z);
  const you = k.box(0.8, 0.8, 0.8, "#16130f", 0.82, GOLD);
  you.position.set(-2.15, 0, 0);
  const lockBox = k.box(0.9, 0.8, 0.8, "#2a2622", 0.9, "#f7931a");
  lockBox.position.set(0, -0.05, 0);
  const maker = k.box(0.8, 0.8, 0.8, "#3a352f", 1, "#a7a098");
  maker.position.set(2.15, 0, 0);
  k.label("you").position.set(-2.15, -0.62, 0.5);
  k.label("BTC lock · 2-of-2", "#f7931a").position.set(0, -0.62, 0.5);
  k.label("maker").position.set(2.15, -0.62, 0.5);
  const zecLock = k.padlock();
  zecLock.group.position.set(0, 1.05, 0);
  k.label("shared ZEC address", GOLD, 0.15).position.set(0, 1.86, 0.3);

  // The block clock: blocks fill the track; T1 and T2 are posts on it.
  const blockX = (i: number) => -2.45 + i * 0.44;
  const blocks = Array.from({ length: 12 }, (_, i) => {
    const b = k.add(new T.Mesh(new T.BoxGeometry(0.3, 0.2, 0.2), new T.MeshStandardMaterial({ color: "#f7931a" })));
    b.position.set(blockX(i), -1.05, 0.2);
    return b;
  });
  const track = k.add(new T.Mesh(new T.BoxGeometry(5.4, 0.02, 0.24), new T.MeshBasicMaterial({ color: "#4a443d" })));
  track.position.set(0, -1.16, 0.2);
  for (const [i, text] of [
    [5, "T1 · cancel"],
    [9, "T2 · punish"],
  ] as const) {
    const post = k.add(new T.Mesh(new T.BoxGeometry(0.03, 0.55, 0.03), new T.MeshBasicMaterial({ color: GOLD })));
    post.position.set(blockX(i) + 0.22, -1.0, 0.32);
    k.label(text, GOLD, 0.15).position.set(blockX(i) + 0.22, -1.42, 0.4);
  }
  k.label("bitcoin blocks →", "#7a706a", 0.13).position.set(-1.95, -1.42, 0.4);

  const zec = k.coin("zec");
  const btc = k.coin("btc");
  const keys = k.keyHalves();
  keys.bow.visible = false;
  const events = {
    wait: k.label("waiting for the maker's BTC…", "#7a706a", 0.17),
    none: k.label("✓ nothing to undo", GREEN, 0.19),
    cancel: k.label("T1 reached → cancel", LEAK, 0.19),
    refund: k.label("refund → maker", INK, 0.17),
    reveal: k.label("refund reveals the maker's half", LEAK, 0.17),
    home: k.label("✓ your ZEC swept home", GREEN, 0.19),
    punish: k.label("T2 reached → penalty to you ✓", GREEN, 0.19),
  };
  for (const e of Object.values(events)) e.position.set(0, 2.32, 0.3);
  const names = Object.keys(events) as (keyof typeof events)[];

  const YOU = V(-2.15, 0, 0.55),
    LOCK = V(0, -0.05, 0.55),
    MAKER = V(2.15, 0, 0.55),
    ZLOCK = V(0, 1.0, 0.35);
  const p = new T.Vector3();
  return (phase, u, t) => {
    const show = (name: keyof typeof events, on: boolean) => (events[name].visible = on);
    for (const n of names) show(n, false);
    zec.rotation.y = t * 1.6;
    btc.rotation.y = t * 1.6 + 1;
    keys.blade.visible = false;
    let ticked = 0;
    let open = 0;

    if (phase === 0) {
      zec.position.copy(YOU);
      btc.position.copy(MAKER);
      ticked = Math.floor(seg(u, 0.1, 0.7) * 6);
      show("wait", u < 0.7);
      show("none", u >= 0.7);
    } else if (phase === 1) {
      btc.position.copy(k.arc(p, MAKER, LOCK, seg(u, 0, 0.2)));
      zec.position.copy(YOU);
      ticked = Math.floor(seg(u, 0.2, 0.6) * 6);
      if (u > 0.62) btc.position.copy(k.arc(p, LOCK, MAKER, seg(u, 0.65, 0.88)));
      show("cancel", u > 0.6 && u < 0.75);
      show("refund", u >= 0.75);
    } else if (phase === 2) {
      btc.position.copy(k.arc(p, MAKER, LOCK, seg(u, 0, 0.12)));
      zec.position.copy(k.arc(p, YOU, ZLOCK, seg(u, 0.12, 0.26), 0.6));
      ticked = Math.floor(seg(u, 0.26, 0.5) * 6);
      if (u > 0.52) btc.position.copy(k.arc(p, LOCK, MAKER, seg(u, 0.52, 0.64)));
      keys.blade.visible = u > 0.62 && u < 0.8;
      keys.blade.position.copy(k.arc(p, MAKER, V(-1.9, 0.75, 0.6), seg(u, 0.62, 0.76), 0.6));
      open = seg(u, 0.76, 0.84);
      if (u > 0.82) zec.position.copy(k.arc(p, ZLOCK, YOU, seg(u, 0.82, 0.96), 0.4));
      show("cancel", u > 0.5 && u < 0.6);
      show("reveal", u >= 0.6 && u < 0.82);
      show("home", u >= 0.82);
    } else {
      btc.position.copy(k.arc(p, MAKER, LOCK, seg(u, 0, 0.1)));
      zec.position.copy(k.arc(p, YOU, ZLOCK, seg(u, 0.1, 0.22), 0.6));
      ticked = Math.floor(seg(u, 0.22, 0.75) * 10);
      if (u > 0.78) btc.position.copy(k.arc(p, LOCK, YOU, seg(u, 0.78, 0.92), 0.7));
      show("cancel", u > 0.42 && u < 0.6);
      show("punish", u >= 0.78);
    }
    zecLock.open(open);
    blocks.forEach((b, i) => {
      b.visible = i < ticked;
      b.scale.setScalar(i === ticked - 1 ? 1.15 : 1);
    });
  };
}

function notes(T: Three, k: Kit): Story {
  const V = (x: number, y: number, z = 0) => new T.Vector3(x, y, z);
  const from = k.box(0.6, 0.6, 0.6, "#16130f", 0.82, GOLD);
  from.position.set(-1.45, -0.35, 0);
  const to = k.box(0.6, 0.6, 0.6, "#16130f", 0.82, GOLD);
  to.position.set(1.45, -0.35, 0);
  k.label("sender", INK, 0.16).position.set(-1.45, -0.82, 0.4);
  k.label("receiver", INK, 0.16).position.set(1.45, -0.82, 0.4);
  const card = k.noteCard();
  const watchers = k.eyes(V(0, 1.05, -0.3));
  const noise = ["▓█░▒█▓░█▒", "░▒█▓░█▓▒░", "█▓░▒░█▒▓█"].map((n) => k.label(n, "#7a706a", 0.2));
  for (const n of noise) n.position.set(0, -0.62, 0.6);
  const ring = new T.Mesh(new T.TorusGeometry(0.17, 0.035, 12, 40), new T.MeshBasicMaterial({ color: GREEN }));
  ring.position.set(0.33, 0.17, 0.03);
  card.add(ring);
  const proof = k.label("proof ✓ valid", GREEN, 0.2);
  proof.position.set(0, 1.0, 0.6);

  const A = V(-1.45, -0.3, 0.3),
    C = V(0, 0.3, 0.4),
    B = V(1.45, -0.3, 0.3);
  const p = new T.Vector3();
  return (phase, u, t) => {
    if (phase === 0) card.position.copy(k.arc(p, A, C, move(u), 0.5));
    else if (phase === 1) card.position.copy(C);
    else card.position.copy(u < 0.5 ? C : k.arc(p, C, B, seg(u, 0.5, 0.85), 0.3));
    const s = phase === 0 ? 0.35 + 0.65 * move(u) : phase === 2 && u > 0.5 ? 1 - 0.6 * seg(u, 0.5, 0.85) : 1;
    card.scale.setScalar(s);
    card.rotation.y = Math.sin(t * 0.8) * 0.18;
    watchers.update(phase === 1 ? move(u) : phase === 2 ? 1 - seg(u, 0.4, 0.6) : 0, card.position);
    noise.forEach((n, i) => (n.visible = phase === 1 && Math.floor(t * 6) % noise.length === i));
    ring.visible = phase === 2;
    ring.scale.setScalar(phase === 2 ? 0.4 + 0.6 * seg(u, 0, 0.25) : 1);
    proof.visible = phase === 2 && u > 0.15;
  };
}

function split(T: Three, k: Kit): Story {
  const V = (x: number, y: number, z = 0) => new T.Vector3(x, y, z);
  const lock = k.padlock();
  lock.group.position.set(0, -0.15, 0);
  lock.group.scale.setScalar(1.3);
  const zec = k.coin("zec");
  zec.scale.setScalar(0.75);
  const { bow, blade } = k.keyHalves();
  const yours = k.label("your half", GOLD, 0.16);
  const makers = k.label("maker's half", LEAK, 0.16);
  const nope = k.label("✗ nothing", LEAK, 0.22);
  nope.position.set(0, 0.95, 0.5);
  const spend = k.label("✓ spendable", GREEN, 0.22);
  spend.position.set(0, 1.25, 0.5);

  const BOW = V(-1.3, 0.55, 0.4),
    BLADE = V(1.3, 0.55, 0.4),
    HOLE = V(0.05, -0.2, 0.6);
  const p = new T.Vector3();
  return (phase, u, t) => {
    const bob = Math.sin(t * 2) * 0.04;
    bow.position.copy(BOW).setY(BOW.y + bob);
    blade.position.copy(BLADE).setY(BLADE.y - bob);
    bow.rotation.x = blade.rotation.x = 0;
    let open = 0;
    zec.position.set(0, -0.1, -0.05);
    zec.rotation.y = t * 1.4;

    if (phase === 1) {
      const go = seg(u, 0, 0.35) * (1 - seg(u, 0.8, 1));
      bow.position.lerpVectors(BOW, HOLE, go);
      if (u > 0.4 && u < 0.8) bow.position.x += Math.sin(t * 40) * 0.035;
    } else if (phase === 2) {
      const go = seg(u, 0, 0.4);
      bow.position.copy(k.arc(p, BOW, HOLE, go, 0.2));
      blade.position.copy(k.arc(p, BLADE, HOLE, go, 0.2));
      bow.rotation.x = blade.rotation.x = seg(u, 0.42, 0.55) * 1.2;
      open = seg(u, 0.55, 0.68);
      const rise = seg(u, 0.65, 0.85);
      zec.position.set(0, -0.1 + rise * 1.05, -0.05 + rise * 0.5);
    }
    lock.open(open);
    yours.visible = makers.visible = !(phase === 2 && u > 0.4);
    yours.position.set(bow.position.x - 0.1, bow.position.y + 0.32, 0.6);
    makers.position.set(blade.position.x + 0.1, blade.position.y + 0.32, 0.6);
    nope.visible = phase === 1 && u > 0.45 && u < 0.85;
    spend.visible = phase === 2 && u > 0.7;
  };
}

function adaptor(T: Three, k: Kit): Story {
  const V = (x: number, y: number, z = 0) => new T.Vector3(x, y, z);
  const btcLock = k.box(0.7, 0.6, 0.6, "#2a2622", 0.9, "#f7931a");
  btcLock.position.set(-1.35, -0.35, 0);
  const zecLock = k.padlock();
  zecLock.group.position.set(1.35, -0.3, 0);
  k.label("BTC lock", "#f7931a", 0.16).position.set(-1.35, -0.85, 0.4);
  k.label("ZEC lock", GOLD, 0.16).position.set(1.35, -0.85, 0.4);
  k.label("you", INK, 0.16).position.set(-1.35, 1.3, 0.4);
  k.label("maker", INK, 0.16).position.set(1.35, 1.3, 0.4);
  const btc = k.coin("btc");
  const zec = k.coin("zec");
  zec.scale.setScalar(0.7);
  const { bow, blade } = k.keyHalves();
  const sig = k.label("signature ✍", INK, 0.17);
  sig.position.set(-1.35, 0.15, 0.6);
  const swept = k.label("✓ maker sweeps ZEC", GREEN, 0.18);
  swept.position.set(0.35, 1.25, 0.6);

  const LOCKB = V(-1.35, -0.3, 0.45),
    YOU = V(-1.35, 0.85, 0.4),
    HALF = V(1.0, 0.45, 0.5),
    HOLE = V(1.38, -0.32, 0.5),
    ZIN = V(1.35, -0.3, -0.05),
    OUT = V(1.75, 0.95, 0.4);
  const p = new T.Vector3();
  return (phase, u, t) => {
    btc.rotation.y = t * 1.6;
    zec.rotation.y = t * 1.4;
    blade.position.copy(HALF);
    bow.rotation.x = blade.rotation.x = 0;
    zec.position.copy(ZIN);
    let open = 0;
    if (phase === 0) {
      btc.position.copy(k.arc(p, LOCKB, YOU, move(u), 0.2));
      bow.visible = u > 0.7;
      bow.position.copy(YOU).add(V(0, -0.45, 0.2));
    } else if (phase === 1) {
      btc.position.copy(YOU);
      bow.visible = true;
      bow.position.copy(k.arc(p, V(-1.35, 0.4, 0.6), HALF, move(u), 0.8));
    } else {
      btc.position.copy(YOU);
      bow.visible = true;
      const go = seg(u, 0, 0.3);
      bow.position.lerpVectors(HALF, HOLE, go);
      blade.position.lerpVectors(HALF, HOLE, go);
      bow.rotation.x = blade.rotation.x = seg(u, 0.3, 0.42) * 1.2;
      open = seg(u, 0.42, 0.55);
      zec.position.copy(k.arc(p, ZIN, OUT, seg(u, 0.55, 0.8), 0.3));
    }
    zecLock.open(open);
    sig.visible = phase === 0 && u > 0.35;
    swept.visible = phase === 2 && u > 0.75;
  };
}

const BUILD: Record<Name, (T: Three, k: Kit) => Story> = { bridge, exchange, pool, safety, notes, split, adaptor };


function mount(T: Three, host: HTMLDivElement, name: Name, onPhase: (p: number) => void) {
  const { ms, w, h, lookY } = CONFIG[name];
  const renderer = new T.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.outputColorSpace = T.SRGBColorSpace;
  renderer.domElement.className = "scene3d-canvas";
  host.prepend(renderer.domElement);

  const scene = new T.Scene();
  const camera = new T.PerspectiveCamera(30, 1, 0.1, 50);
  scene.add(new T.AmbientLight(0xffffff, 1.2));
  const keyLight = new T.DirectionalLight(0xffffff, 2.4);
  keyLight.position.set(3, 5, 5);
  const rim = new T.DirectionalLight(0xf4b728, 1.4);
  rim.position.set(-4, 2, -3);
  scene.add(keyLight, rim);

  const story = BUILD[name](T, kit(T, scene));
  const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const steps = STORY[name].length;

  const resize = () => {
    // The canvas, not the host: CSS leaves room in the host for captions.
    const { width, height } = renderer.domElement.getBoundingClientRect();
    if (width < 1 || height < 1) return;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    // Back off until the scene's whole area fits the card.
    const half = 2 * Math.tan((camera.fov * Math.PI) / 360);
    const dist = Math.max(h / half, w / (half * camera.aspect)) + 1.2;
    camera.position.set(0, lookY + 1.6, dist);
    camera.lookAt(0, lookY, 0);
    camera.updateProjectionMatrix();
  };
  const ro = new ResizeObserver(resize);
  ro.observe(host);
  ro.observe(renderer.domElement);
  resize();

  let raf = 0;
  let start = performance.now();
  let lastPhase = -1;
  let visible = false;
  const frame = (now: number) => {
    const elapsed = now - start;
    const phase = Math.floor(elapsed / ms) % steps;
    const u = (elapsed % ms) / ms;
    if (phase !== lastPhase) onPhase((lastPhase = phase));
    story(phase, u, elapsed / 1000);
    camera.position.x = Math.sin(elapsed / 4000) * 0.35;
    camera.lookAt(0, lookY, 0);
    renderer.render(scene, camera);
    raf = requestAnimationFrame(frame);
  };
  const io = new IntersectionObserver(([e]) => {
    cancelAnimationFrame(raf);
    visible = e.isIntersecting;
    if (!visible) return;
    if (still) {
      onPhase(steps - 1);
      story(steps - 1, 1, 0);
      renderer.render(scene, camera);
      return;
    }
    start = performance.now();
    lastPhase = -1;
    raf = requestAnimationFrame(frame);
  });
  io.observe(host);

  const jump = (phase: number) => {
    start = performance.now() - phase * ms;
    lastPhase = -1;
  };

  const dispose = () => {
    cancelAnimationFrame(raf);
    io.disconnect();
    ro.disconnect();
    scene.traverse((o) => {
      const m = o as import("three").Mesh;
      m.geometry?.dispose();
      const mats = Array.isArray(m.material) ? m.material : m.material ? [m.material] : [];
      for (const mat of mats) {
        (mat as import("three").MeshBasicMaterial).map?.dispose();
        mat.dispose();
      }
    });
    renderer.dispose();
    renderer.domElement.remove();
  };
  return { dispose, jump };
}

/**
 * `card`: under a folder tab, with a caption box. `full`: fills its parent, caption box, steps
 * clickable. `compact`: a one-line caption at the top, for small cards.
 */
export function RouteScene({
  name,
  variant = "card",
  steps,
}: {
  name: Name;
  variant?: "card" | "full" | "compact";
  steps?: string[];
}) {
  const host = useRef<HTMLDivElement>(null);
  const control = useRef<{ dispose: () => void; jump: (p: number) => void } | undefined>(undefined);
  const [phase, setPhase] = useState(0);

  useEffect(() => {
    let cancelled = false;
    // Labels are drawn into canvases, which don't trigger a webfont load on their own.
    Promise.all([import("three"), document.fonts?.load(`500 64px "JetBrains Mono Variable"`)]).then(([T]) => {
      if (!cancelled && host.current) control.current = mount(T, host.current, name, setPhase);
    });
    return () => {
      cancelled = true;
      control.current?.dispose();
    };
  }, [name]);

  const lines = STORY[name];
  const counter = `${String(phase + 1).padStart(2, "0")}/${String(lines.length).padStart(2, "0")}`;
  if (variant === "compact") {
    return (
      <div className="scene3d is-compact" ref={host}>
        <p className="scene-line" aria-live="polite">
          <span className="scene-step">{counter}</span> {lines[phase]}
        </p>
      </div>
    );
  }
  return (
    <div className={`scene3d is-${variant}`} ref={host}>
      <div className="scene-caption" aria-live="polite">
        <span className="scene-step">{counter}</span>
        <p>{lines[phase]}</p>
        <span className="scene-dots">
          {lines.map((_, i) =>
            variant === "full" ? (
              <button
                type="button"
                key={i}
                className={i === phase ? "on" : undefined}
                aria-label={steps?.[i] ?? `Step ${i + 1}`}
                onClick={() => control.current?.jump(i)}
              >
                {steps?.[i]}
              </button>
            ) : (
              <i key={i} className={i === phase ? "on" : undefined} />
            ),
          )}
        </span>
      </div>
    </div>
  );
}
