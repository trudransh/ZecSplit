import type { Network } from "./engine";

const COIN = 100_000_000;

/** 8-decimal amount without trailing zeros: 12.5, 0.0461. */
export const coin = (units: number) => (units / COIN).toFixed(8).replace(/\.?0+$/, "");

/** "1.25" → 125000000 zatoshis; null if it isn't a positive amount with at most 8 decimals. */
export function parseCoin(text: string): number | null {
  const m = text.trim().match(/^(\d+)(?:\.(\d{0,8}))?$/);
  if (!m) return null;
  const units = Number(m[1]) * COIN + Number((m[2] ?? "").padEnd(8, "0"));
  return units > 0 && Number.isSafeInteger(units) ? units : null;
}

/** UX_SPEC §1.2: absolute local time with the zone, UTC on hover. */
export function when(unix: number) {
  const d = new Date(unix * 1000);
  return {
    // dateStyle/timeStyle can't be combined with timeZoneName, so the fields are spelled out.
    local: d.toLocaleString(undefined, {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
      timeZoneName: "short",
    }),
    utc: `${d.toISOString().slice(0, 16).replace("T", " ")} UTC`,
  };
}

/** UX_SPEC §4: hashes shown middle-truncated, always copied in full. */
export const short = (h: string, n = 8) => (h.length > n * 2 + 1 ? `${h.slice(0, n)}…${h.slice(-n)}` : h);

export function explorer(network: Network, txid: string): string | null {
  if (network === "test") return `https://mempool.space/signet/tx/${txid}`;
  if (network === "main_beta") return `https://mempool.space/tx/${txid}`;
  return null;
}

/** The address prefix this network expects, for a friendly hint before the engine checks it. */
export const btcPrefix = (network: Network) => (network === "regtest" ? "bcrt1" : network === "test" ? "tb1" : "bc1");

/** A rough passphrase strength from length and character variety, 0 to 4. */
export function strength(p: string) {
  const kinds = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(p)).length;
  return Math.min(4, (p.length >= 10 ? 1 : 0) + (p.length >= 16 ? 1 : 0) + (kinds >= 3 ? 1 : 0) + (p.length >= 24 ? 1 : 0));
}
