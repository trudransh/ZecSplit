// The swap page's only contract with the engine (INTERFACES §3). Types come from ts-rs.
import type { CreateSwapRequest } from "../../../../packages/engine-api/src/CreateSwapRequest";
import type { CreateSwapResponse } from "../../../../packages/engine-api/src/CreateSwapResponse";
import type { Health } from "../../../../packages/engine-api/src/Health";
import type { QuoteView } from "../../../../packages/engine-api/src/QuoteView";
import type { RestoreRequest } from "../../../../packages/engine-api/src/RestoreRequest";
import type { Step } from "../../../../packages/engine-api/src/Step";
import type { StepStatus } from "../../../../packages/engine-api/src/StepStatus";
import type { SwapSummary } from "../../../../packages/engine-api/src/SwapSummary";
import type { SwapView } from "../../../../packages/engine-api/src/SwapView";

export type { Health, QuoteView, Step, SwapSummary, SwapView };
export type { ErrorCode } from "../../../../packages/engine-api/src/ErrorCode";
export type { Network } from "../../../../packages/engine-api/src/Network";
export type { StepKind } from "../../../../packages/engine-api/src/StepKind";
export type { StepStatus };

export interface Engine {
  readonly demo: boolean;
  health(): Promise<Health>;
  unlock(passphrase: string): Promise<void>;
  quote(zatoshis: number): Promise<QuoteView>;
  createSwap(req: CreateSwapRequest): Promise<CreateSwapResponse>;
  swaps(): Promise<SwapSummary[]>;
  swap(id: string): Promise<SwapView>;
  /** Pushes the whole view each time it changes; returns the unsubscribe. */
  watch(id: string, on: (view: SwapView) => void): () => void;
  /** The sealed recovery file as base64 text, or null until setup completes. */
  recoveryFile(id: string): Promise<string | null>;
  recoverySaved(id: string): Promise<void>;
  restore(req: RestoreRequest): Promise<CreateSwapResponse>;
  /** Demo only: stands in for the user's Zcash wallet paying the QR. */
  simulatePayment?(id: string): void;
}

export class EngineError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

export class HttpEngine implements Engine {
  readonly demo = false;

  constructor(
    private base: string,
    private token: string,
  ) {}

  private async call<T>(method: string, path: string, body?: unknown): Promise<T> {
    let res: Response;
    try {
      res = await fetch(this.base + path, {
        method,
        headers: { authorization: `Bearer ${this.token}`, "content-type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch {
      throw new EngineError("ENGINE_OFFLINE", "Can't reach the swap engine on this computer.");
    }
    if (res.status === 204) return undefined as T;
    if (!res.ok) {
      const text = await res.text();
      try {
        const e = JSON.parse(text) as { code: string; message: string };
        throw new EngineError(e.code, e.message);
      } catch (err) {
        if (err instanceof EngineError) throw err;
        throw new EngineError(res.status === 401 ? "unauthorized" : `http_${res.status}`, text || res.statusText);
      }
    }
    const type = res.headers.get("content-type") ?? "";
    return (type.includes("json") ? res.json() : res.text()) as Promise<T>;
  }

  health() {
    return this.call<Health>("GET", "/api/health");
  }
  unlock(passphrase: string) {
    return this.call<void>("POST", "/api/unlock", { passphrase });
  }
  quote(zatoshis: number) {
    return this.call<QuoteView>("POST", "/api/quote", { zec_amount: zatoshis });
  }
  createSwap(req: CreateSwapRequest) {
    return this.call<CreateSwapResponse>("POST", "/api/swaps", req);
  }
  swaps() {
    return this.call<SwapSummary[]>("GET", "/api/swaps");
  }
  swap(id: string) {
    return this.call<SwapView>("GET", `/api/swaps/${id}`);
  }
  watch(id: string, on: (view: SwapView) => void) {
    // EventSource can't set headers, so the token rides in the query (INTERFACES §3).
    const es = new EventSource(`${this.base}/api/swaps/${id}/events?token=${encodeURIComponent(this.token)}`);
    es.onmessage = (e) => on(JSON.parse(e.data) as SwapView);
    return () => es.close();
  }
  async recoveryFile(id: string) {
    try {
      return await this.call<string>("GET", `/api/swaps/${id}/recovery-file`);
    } catch (e) {
      if (e instanceof EngineError && e.code === "not_ready") return null;
      throw e;
    }
  }
  recoverySaved(id: string) {
    return this.call<void>("POST", `/api/swaps/${id}/recovery-saved`);
  }
  restore(req: RestoreRequest) {
    return this.call<CreateSwapResponse>("POST", "/api/restore", req);
  }
}

const hex = (bytes: number) =>
  Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (b) => b.toString(16).padStart(2, "0")).join("");

interface DemoSwap {
  view: SwapView;
  created: number;
  paidAt: number | null;
  lockTxid: string;
  redeemTxid: string;
}

/**
 * An in-browser stand-in that walks the same view model through a happy-path swap in about
 * 40 seconds. Nothing touches a chain; the page labels it as a demo everywhere.
 */
export class DemoEngine implements Engine {
  readonly demo = true;
  private unlocked = false;
  private quotes = new Map<string, QuoteView>();
  private swapsById = new Map<string, DemoSwap>();
  private listeners = new Map<string, Set<(v: SwapView) => void>>();
  private timer = 0;

  private now() {
    return Math.floor(Date.now() / 1000);
  }

  async health(): Promise<Health> {
    return { version: "demo", network: "test", unlocked: this.unlocked };
  }

  async unlock(passphrase: string) {
    if (passphrase.length < 10) throw new EngineError("weak_passphrase", "use at least 10 characters");
    this.unlocked = true;
  }

  async quote(zatoshis: number): Promise<QuoteView> {
    const min = 1_000_000;
    const max = 200_000_000;
    if (zatoshis < min || zatoshis > max) {
      throw new EngineError("maker", "maker: quote_refused: amount outside the limits");
    }
    const now = this.now();
    const q: QuoteView = {
      quote_id: hex(32),
      zec_amount: zatoshis,
      btc_amount: Math.floor((zatoshis * 500_000) / 100_000_000),
      expires_at: now + 60,
      min_zec: min,
      max_zec: max,
      refund_window: { opens_at: now + 37 * 600, closes_at: now + 73 * 600 },
    };
    this.quotes.set(q.quote_id, q);
    return q;
  }

  async createSwap(req: { quote_id: string; btc_address: string; accept_risks: boolean }) {
    if (!this.unlocked) throw new EngineError("locked", "unlock the engine first");
    if (!req.accept_risks) throw new EngineError("risks_not_accepted", "accept the risks first");
    const q = this.quotes.get(req.quote_id);
    if (!q || q.expires_at <= this.now()) throw new EngineError("quote_expired", "no such quote, or it expired");
    if (!/^(tb1|bcrt1)[a-z0-9]{20,80}$/.test(req.btc_address)) {
      throw new EngineError("bad_btc_address", "not a Bitcoin address on this network");
    }
    this.quotes.delete(req.quote_id);
    const steps: Step[] = (["setup", "maker_locks_btc", "pay_zec", "maker_verifies", "claim_btc", "done"] as const).map(
      (kind) => ({ kind, status: "pending", confirmations: null, txid: null, deadline: null }),
    );
    this.swapsById.set(q.quote_id, {
      created: Date.now(),
      paidAt: null,
      lockTxid: hex(32),
      redeemTxid: hex(32),
      view: {
        swap_id: q.quote_id,
        network: "test",
        zec_amount: q.zec_amount,
        btc_amount: q.btc_amount,
        btc_address: req.btc_address,
        steps,
        zip321_uri: null,
        refund_window: q.refund_window,
        recovery_saved: false,
        outcome: null,
        error: null,
      },
    });
    this.tick();
    if (!this.timer) this.timer = window.setInterval(() => this.tick(), 1000);
    return { swap_id: q.quote_id };
  }

  async swaps(): Promise<SwapSummary[]> {
    return [...this.swapsById.values()].map((s) => ({
      swap_id: s.view.swap_id,
      created_at: Math.floor(s.created / 1000),
      zec_amount: s.view.zec_amount,
      btc_amount: s.view.btc_amount,
      current_step: s.view.steps.find((st) => st.status !== "confirmed")?.kind ?? "done",
      outcome: s.view.outcome,
    }));
  }

  async swap(id: string) {
    const s = this.swapsById.get(id);
    if (!s) throw new EngineError("unknown_swap", "no such swap");
    return structuredClone(s.view);
  }

  watch(id: string, on: (v: SwapView) => void) {
    const set = this.listeners.get(id) ?? new Set();
    set.add(on);
    this.listeners.set(id, set);
    const s = this.swapsById.get(id);
    if (s) on(structuredClone(s.view));
    return () => set.delete(on);
  }

  async recoveryFile(id: string) {
    const s = this.swapsById.get(id);
    if (!s || s.view.steps[0].status !== "confirmed") return null;
    return btoa(`demo recovery file for swap ${id} (not a real file)`);
  }

  async recoverySaved(id: string) {
    const s = this.swapsById.get(id);
    if (s) s.view.recovery_saved = true;
    this.tick();
  }

  async restore(): Promise<{ swap_id: string }> {
    throw new EngineError("bad_recovery_file", "Restore needs the real engine; the demo has nothing to restore.");
  }

  simulatePayment(id: string) {
    const s = this.swapsById.get(id);
    if (s && s.view.zip321_uri && !s.paidAt) s.paidAt = Date.now();
    this.tick();
  }

  /** Mirrors the engine's step statuses for each stage of a happy-path swap. */
  private tick() {
    for (const s of this.swapsById.values()) {
      const v = s.view;
      if (v.outcome) continue;
      const t = (Date.now() - s.created) / 1000;
      const paid = s.paidAt ? (Date.now() - s.paidAt) / 1000 : -1;
      const C: StepStatus = "confirmed";
      const set = (st: StepStatus[]) => st.forEach((x, i) => (v.steps[i].status = x));
      const lock = v.steps[1];
      const claim = v.steps[4];
      const base = this.now();
      const deadline = { height: 1_204_812, estimated_at: base + 30 * 600 };

      if (t < 4) set(["waiting", "pending", "pending", "pending", "pending", "pending"]);
      else if (t < 8) set([C, "waiting", "pending", "pending", "pending", "pending"]);
      else if (t < 12) {
        set([C, "seen", "pending", "pending", "pending", "pending"]);
        lock.txid = s.lockTxid;
        lock.confirmations = { have: 0, need: 1 };
      } else if (paid < 0) {
        set([C, C, "waiting", "pending", "pending", "pending"]);
        lock.confirmations = { have: 1, need: 1 };
        v.steps[2].deadline = deadline;
        v.zip321_uri = v.recovery_saved
          ? `zcash:utest1demo${s.lockTxid.slice(0, 40)}?amount=${(v.zec_amount / 1e8).toString()}`
          : null;
      } else if (paid < 5) {
        set([C, C, C, "waiting", "pending", "pending"]);
        v.zip321_uri = null;
      } else if (paid < 9) {
        set([C, C, C, C, "waiting", "pending"]);
        claim.deadline = deadline;
      } else if (paid < 13) {
        set([C, C, C, C, "seen", "pending"]);
        claim.txid = s.redeemTxid;
        claim.confirmations = { have: 0, need: 1 };
      } else {
        set([C, C, C, C, C, C]);
        claim.confirmations = { have: 1, need: 1 };
        v.outcome = "redeemed";
      }
      this.listeners.get(v.swap_id)?.forEach((on) => on(structuredClone(v)));
    }
  }
}
