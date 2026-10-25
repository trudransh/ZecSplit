"use client";

import { useEffect, useRef } from "react";
import { AsciiCoins, COINS } from "./ascii-coins";

const pct = (v: number) => `${(v * 100).toFixed(1)}%`;

export function HeroArt() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    if (!ref.current) return;
    const art = new AsciiCoins(ref.current);
    return () => art.destroy();
  }, []);

  return (
    <div className="hero-art">
      <canvas ref={ref} aria-hidden="true" className="hero-canvas" />
      <span className="coin-label" style={{ left: pct(COINS.zec.fx - COINS.zec.fr), top: pct(COINS.zec.fy - COINS.zec.fr - 0.07) }}>
        <i className="sq sq-zec" /> ZEC · shielded pool
      </span>
      <span className="coin-label" style={{ right: pct(1 - COINS.btc.fx - COINS.btc.fr), top: pct(COINS.btc.fy + COINS.btc.fr + 0.03) }}>
        <i className="sq sq-btc" /> BTC · 2-of-2 script
      </span>
    </div>
  );
}

// Deadlines live on Bitcoin (PROTOCOL_SPEC §4.4, §5.3), so the ruler counts blocks, not hours.
export function Ruler() {
  return (
    <div className="ruler" aria-hidden="true">
      <span className="ruler-now">NOW</span>
      {["LOCK", "CLAIM", "T1 · CANCEL", "REFUND", "T2 · PUNISH"].map((l, i) => (
        <span className="ruler-mark" key={l} style={{ top: `${8 + i * 21}%` }}>
          {l}
        </span>
      ))}
    </div>
  );
}
