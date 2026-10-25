"use client";

import { useEffect, useRef } from "react";
import { OceanEngine } from "./ocean/engine";
import type { SharkSpec } from "./ocean/world";

// A mixed school like the author's demo video; the first is the biggest.
const SHARKS: SharkSpec[] = [
  { kind: "greathammer", size: 1 },
  { kind: "tiger", size: 0.9 },
  { kind: "classic", size: 0.84 },
  { kind: "mako", size: 0.78 },
];

/** stackedlol/ascii-sharks, swimming in the footer. */
export function Ocean({ className }: { className?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    if (!ref.current) return;
    const engine = new OceanEngine(ref.current, SHARKS);
    return () => engine.destroy();
  }, []);

  return <canvas ref={ref} aria-hidden="true" className={className} />;
}
