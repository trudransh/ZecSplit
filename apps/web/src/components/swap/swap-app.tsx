"use client";

import { useCallback, useEffect, useState } from "react";
import {
  DemoEngine,
  type Engine,
  EngineError,
  type Health,
  HttpEngine,
  type QuoteView,
  type SwapSummary,
  type SwapView,
} from "@/lib/engine";
import { btcPrefix, coin, parseCoin, short, strength, when } from "@/lib/format";
import { SwapProgress } from "./progress";

type Stage = "connect" | "unlock" | "quote" | "address" | "risks" | "progress" | "restore";

const STAGES: [Stage, string][] = [
  ["connect", "Connect"],
  ["unlock", "Passphrase"],
  ["quote", "Quote"],
  ["address", "BTC address"],
  ["risks", "Before you start"],
  ["progress", "Swap"],
];

const SAVED = "zecsplit.engine";
const DEFAULT_ENGINE = "http://127.0.0.1:7780";

function message(e: unknown) {
  if (e instanceof EngineError) {
    if (e.code === "ENGINE_OFFLINE") {
      return "Can't reach the swap engine on this computer. Start it with zecsplitd, then open the link it prints.";
    }
    if (e.code === "unauthorized") return "The engine refused the session token. Open the link zecsplitd printed again.";
    return e.message.replace(/^maker: (quote_refused: )?/, "");
  }
  return String(e);
}

function Countdown({ until }: { until: number }) {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(t);
  }, []);
  const left = Math.max(0, Math.floor(until - now));
  return (
    <span className={left < 10 ? "countdown low" : "countdown"} role="timer">
      {left > 0 ? `Quote valid for ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}` : "Quote expired"}
    </span>
  );
}

export function SwapApp() {
  const [engine, setEngine] = useState<Engine | null>(null);
  const [health, setHealth] = useState<Health | null>(null);
  const [stage, setStage] = useState<Stage>("connect");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const [base, setBase] = useState(DEFAULT_ENGINE);
  const [token, setToken] = useState("");
  const [pass, setPass] = useState("");
  const [amount, setAmount] = useState("");
  const [quote, setQuote] = useState<QuoteView | null>(null);
  const [address, setAddress] = useState("");
  const [accepted, setAccepted] = useState(false);
  const [active, setActive] = useState<SwapSummary[]>([]);
  const [swapId, setSwapId] = useState<string | null>(null);
  const [view, setView] = useState<SwapView | null>(null);
  const [restoreFile, setRestoreFile] = useState("");
  const [restorePass, setRestorePass] = useState("");

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await f();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };

  const open = useCallback((id: string) => {
    setSwapId(id);
    setStage("progress");
    const url = new URL(location.href);
    url.searchParams.set("id", id);
    history.replaceState(null, "", url);
  }, []);

  /** After connecting or unlocking: resume a swap in the URL, or offer unfinished ones. */
  const enter = useCallback(
    async (e: Engine) => {
      const h = await e.health();
      setHealth(h);
      if (!h.unlocked) return setStage("unlock");
      const list = await e.swaps();
      setActive(list.filter((s) => !s.outcome));
      const id = new URLSearchParams(location.search).get("id");
      if (id && list.some((s) => s.swap_id === id)) open(id);
      else setStage("quote");
    },
    [open],
  );

  const connect = useCallback(
    (e: Engine) =>
      run(async () => {
        await enter(e);
        setEngine(e);
      }),
    [enter],
  );

  useEffect(() => {
    const hash = new URLSearchParams(location.hash.slice(1));
    const params = new URLSearchParams(location.search);
    let saved: { base: string; token: string } | null = null;
    try {
      if (hash.get("token")) {
        saved = { base: hash.get("engine") || DEFAULT_ENGINE, token: hash.get("token") ?? "" };
        sessionStorage.setItem(SAVED, JSON.stringify(saved));
        history.replaceState(null, "", location.pathname + location.search);
      } else {
        saved = JSON.parse(sessionStorage.getItem(SAVED) ?? "null");
      }
    } catch {}
    if (params.get("demo") === "1") void connect(new DemoEngine());
    else if (saved) {
      setBase(saved.base);
      setToken(saved.token);
      void connect(new HttpEngine(saved.base, saved.token));
    }
  }, [connect]);

  useEffect(() => {
    if (!engine || !swapId) return;
    setView(null);
    return engine.watch(swapId, setView);
  }, [engine, swapId]);

  const zats = parseCoin(amount);
  const prefix = btcPrefix(health?.network ?? "test");
  const addressOk = address.startsWith(prefix) && address.length >= prefix.length + 20;
  const score = strength(pass);
  const current = STAGES.findIndex(([s]) => s === stage);

  return (
    <div className="swap-grid">
      <section className="swap-main" aria-live="polite">
        {engine?.demo && (
          <p className="demo-banner">
            <i className="led led-bad" /> Demo mode: simulated in your browser. No real coins move.
          </p>
        )}
        {active.length > 0 && stage !== "progress" && (
          <div className="resume">
            <span>
              <strong>Swap in progress:</strong> {coin(active[0].zec_amount)} ZEC → {coin(active[0].btc_amount)} BTC
            </span>
            <button type="button" className="btn btn-dark btn-sm" onClick={() => open(active[0].swap_id)}>
              Resume <span className="arrow">→</span>
            </button>
          </div>
        )}
        {error && (
          <div className="swap-alert" role="alert">
            {error}
          </div>
        )}

        {stage === "connect" && (
          <div className="swap-panel">
            <span className="mono-label">01 · Connect</span>
            <h3>Connect your swap engine.</h3>
            <p>
              The engine runs on your own computer and holds your keys; this page is only the screen. Start{" "}
              <code>zecsplitd</code> and open the link it prints, or paste its address and session token here.
            </p>
            <form
              className="form"
              onSubmit={(e) => {
                e.preventDefault();
                try {
                  sessionStorage.setItem(SAVED, JSON.stringify({ base, token }));
                } catch {}
                void connect(new HttpEngine(base.replace(/\/$/, ""), token.trim()));
              }}
            >
              <label>
                <span className="mono-label">Engine address</span>
                <input value={base} onChange={(e) => setBase(e.target.value)} spellCheck={false} />
              </label>
              <label>
                <span className="mono-label">Session token</span>
                <input
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  placeholder="from the ui-token file in the engine's data folder"
                  spellCheck={false}
                  autoComplete="off"
                />
              </label>
              <div className="row-actions">
                <button className="btn btn-dark" disabled={busy || !token.trim()}>
                  Connect <span className="arrow">→</span>
                </button>
                <button type="button" className="btn btn-line" onClick={() => void connect(new DemoEngine())}>
                  Try the demo
                </button>
              </div>
            </form>
          </div>
        )}

        {stage === "unlock" && engine && (
          <form
            className="swap-panel"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                await engine.unlock(pass);
                setPass("");
                await enter(engine);
              });
            }}
          >
            <span className="mono-label">02 · Passphrase</span>
            <h3>Choose a passphrase.</h3>
            <p>
              It unlocks your saved swaps and encrypts your recovery file. Use at least 10 characters, and don&apos;t
              lose it: without it, the recovery file can&apos;t be opened.
            </p>
            <label className="form">
              <span className="mono-label">Passphrase</span>
              <input
                type="password"
                value={pass}
                onChange={(e) => setPass(e.target.value)}
                autoComplete="new-password"
                minLength={10}
                required
              />
            </label>
            <div className={`meter m${score}`} aria-label={`Strength ${score} of 4`}>
              <i />
              <i />
              <i />
              <i />
            </div>
            <div className="row-actions">
              <button className="btn btn-dark" disabled={busy || pass.length < 10}>
                Unlock <span className="arrow">→</span>
              </button>
              <button type="button" className="btn btn-line" onClick={() => setStage("restore")}>
                Restore from a recovery file
              </button>
            </div>
          </form>
        )}

        {stage === "quote" && engine && (
          <div className="swap-panel">
            <span className="mono-label">03 · Quote</span>
            <h3>How much ZEC do you want to swap?</h3>
            <form
              className="amount-row"
              onSubmit={(e) => {
                e.preventDefault();
                if (zats) void run(async () => setQuote(await engine.quote(zats)));
              }}
            >
              <label className="amount">
                <input
                  value={amount}
                  onChange={(e) => {
                    setAmount(e.target.value);
                    setQuote(null);
                  }}
                  inputMode="decimal"
                  placeholder="0.00"
                  aria-label="ZEC amount"
                  autoFocus
                />
                <span>ZEC</span>
              </label>
              <button className="btn btn-dark" disabled={busy || !zats}>
                Get quote
              </button>
            </form>
            {quote && (
              <div className="quote">
                <div className="quote-row">
                  <span>You send</span>
                  <strong>{coin(quote.zec_amount)} ZEC</strong>
                </div>
                <div className="quote-row">
                  <span>You get</span>
                  <strong className="big">{coin(quote.btc_amount)} BTC</strong>
                </div>
                <div className="quote-row">
                  <span>Rate</span>
                  <span>1 ZEC = {coin(Math.round((quote.btc_amount * 1e8) / quote.zec_amount))} BTC</span>
                </div>
                <div className="quote-row muted">
                  <span>Limits</span>
                  <span>
                    Min {coin(quote.min_zec)} · Max {coin(quote.max_zec)} ZEC
                    {health?.network === "test" ? " (testnet)" : ""}
                  </span>
                </div>
                <div className="quote-foot">
                  <Countdown until={quote.expires_at} />
                  <button type="button" className="btn btn-dark" onClick={() => setStage("address")}>
                    Continue <span className="arrow">→</span>
                  </button>
                </div>
              </div>
            )}
            <button type="button" className="link-btn" onClick={() => setStage("restore")}>
              Restore a swap from a recovery file
            </button>
          </div>
        )}

        {stage === "address" && quote && (
          <form
            className="swap-panel"
            onSubmit={(e) => {
              e.preventDefault();
              setStage("risks");
            }}
          >
            <span className="mono-label">04 · BTC address</span>
            <h3>Where should the BTC go?</h3>
            <p>
              Use a fresh address from your own Bitcoin wallet. It will be public on Bitcoin, like any Bitcoin
              payment.
            </p>
            <label className="form">
              <span className="mono-label">
                Bitcoin address ({health?.network === "regtest" ? "regtest" : "signet"}, starts with {prefix})
              </span>
              <input value={address} onChange={(e) => setAddress(e.target.value.trim())} spellCheck={false} required />
            </label>
            {address && !addressOk && <p className="hint">That doesn&apos;t look like a {prefix}… address.</p>}
            <div className="row-actions">
              <button className="btn btn-dark" disabled={!addressOk}>
                Continue <span className="arrow">→</span>
              </button>
              <button type="button" className="btn btn-line" onClick={() => setStage("quote")}>
                Back
              </button>
            </div>
          </form>
        )}

        {stage === "risks" && quote && engine && (
          <div className="swap-panel">
            <span className="mono-label">05 · Before you start</span>
            <h3>Read this once.</h3>
            <ul className="risks">
              <li>
                This swap takes{" "}
                <strong>{health?.network === "regtest" ? "a few minutes on this local test chain" : "about 45 to 90 minutes"}</strong>.
              </li>
              <li>
                If the maker disappears, your ZEC comes back automatically, but the engine must be running at some
                point between <strong title={when(quote.refund_window.opens_at).utc}>{when(quote.refund_window.opens_at).local}</strong>{" "}
                and <strong title={when(quote.refund_window.closes_at).utc}>{when(quote.refund_window.closes_at).local}</strong>.
              </li>
              <li>Save your recovery file when it&apos;s offered. It&apos;s the only way to recover on another device.</li>
              <li>ZecSplit is experimental software and hasn&apos;t been audited. Testnet only.</li>
            </ul>
            <div className="quote compact">
              <div className="quote-row">
                <span>Swap</span>
                <strong>
                  {coin(quote.zec_amount)} ZEC → {coin(quote.btc_amount)} BTC
                </strong>
              </div>
              <div className="quote-row">
                <span>To</span>
                <code>{short(address, 10)}</code>
              </div>
            </div>
            <label className="check">
              <input type="checkbox" checked={accepted} onChange={(e) => setAccepted(e.target.checked)} />
              <span>I understand.</span>
            </label>
            <div className="row-actions">
              <button
                type="button"
                className="btn btn-dark"
                disabled={!accepted || busy}
                onClick={() =>
                  void run(async () => {
                    try {
                      const r = await engine.createSwap({ quote_id: quote.quote_id, btc_address: address, accept_risks: true });
                      open(r.swap_id);
                    } catch (e) {
                      if (e instanceof EngineError && e.code === "quote_expired") {
                        setQuote(null);
                        setStage("quote");
                      }
                      throw e;
                    }
                  })
                }
              >
                Start swap <span className="arrow">→</span>
              </button>
              <button type="button" className="btn btn-line" onClick={() => setStage("address")}>
                Back
              </button>
            </div>
          </div>
        )}

        {stage === "restore" && engine && (
          <form
            className="swap-panel"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                if (!(await engine.health()).unlocked) throw new EngineError("locked", "Choose a passphrase first; it protects the restored swap.");
                const r = await engine.restore({ recovery_file: restoreFile.trim(), passphrase: restorePass });
                open(r.swap_id);
              });
            }}
          >
            <span className="mono-label">Restore</span>
            <h3>Finish a swap from its recovery file.</h3>
            <p>Upload the file or paste its text, and enter the passphrase it was saved with.</p>
            <label className="form">
              <span className="mono-label">Recovery file</span>
              <input
                type="file"
                accept=".recovery,text/plain"
                onChange={(e) => void e.target.files?.[0]?.text().then(setRestoreFile)}
              />
              <textarea value={restoreFile} onChange={(e) => setRestoreFile(e.target.value)} rows={4} spellCheck={false} />
            </label>
            <label className="form">
              <span className="mono-label">Passphrase</span>
              <input type="password" value={restorePass} onChange={(e) => setRestorePass(e.target.value)} />
            </label>
            <div className="row-actions">
              <button className="btn btn-dark" disabled={busy || !restoreFile.trim() || !restorePass}>
                Restore <span className="arrow">→</span>
              </button>
              <button type="button" className="btn btn-line" onClick={() => setStage(health?.unlocked ? "quote" : "unlock")}>
                Back
              </button>
            </div>
          </form>
        )}

        {stage === "progress" && engine && view && <SwapProgress engine={engine} view={view} />}
        {stage === "progress" && !view && (
          <p className="status-line" role="status">
            <i className="spinner" /> Loading the swap…
          </p>
        )}
      </section>

      <aside className="swap-side">
        <div className="side-box">
          <span className="mono-label">Your swap</span>
          <ol className="rail-mini">
            {STAGES.map(([s, label], i) => (
              <li key={s} className={i < current ? "done" : i === current ? "now" : undefined}>
                <span>{String(i + 1).padStart(2, "0")}</span> {label}
              </li>
            ))}
          </ol>
        </div>
        <div className="side-box">
          <span className="mono-label">What stays private</span>
          <ul className="side-list">
            <li>Your ZEC never touches a transparent address.</li>
            <li>On Zcash, the swap is two ordinary shielded transactions.</li>
            <li>The maker never holds your coins: every deadline is a Bitcoin timelock.</li>
          </ul>
        </div>
        {health && (
          <div className="side-box">
            <span className="mono-label">Engine</span>
            <p className="side-meta">
              <i className="led led-good" /> {engine?.demo ? "Demo" : `zecsplitd ${health.version}`} ·{" "}
              {health.network === "test" ? "testnet + signet" : health.network}
            </p>
          </div>
        )}
      </aside>
    </div>
  );
}
