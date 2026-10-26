"use client";

import QRCode from "qrcode";
import { useEffect, useState } from "react";
import type { Engine, ErrorCode, Step, StepKind, SwapView } from "@/lib/engine";
import { coin, explorer, short, when } from "@/lib/format";

const STEP_TEXT: Record<StepKind, [string, string]> = {
  setup: ["Setup", "Your engine and the maker agree on the swap and exchange their halves of the keys."],
  maker_locks_btc: ["Maker locks BTC", "The maker locks the BTC on Bitcoin first. You pay nothing until it's confirmed."],
  pay_zec: ["You pay ZEC", "Pay the QR code from your own Zcash wallet. The payment stays shielded."],
  maker_verifies: ["Maker verifies", "The maker sees the ZEC arrive and sends what you need to claim the BTC."],
  claim_btc: ["Claim BTC", "Your engine claims the BTC to your address."],
  done: ["Done", "The maker sweeps the ZEC. The swap is complete."],
};

function Copy({ text, label = "Copy" }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className="mini-btn"
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setDone(true);
          setTimeout(() => setDone(false), 2000);
        });
      }}
    >
      {done ? "Copied" : label}
    </button>
  );
}

function At({ unix }: { unix: number }) {
  const t = when(unix);
  return (
    <time dateTime={new Date(unix * 1000).toISOString()} title={t.utc}>
      {t.local}
    </time>
  );
}

function statusText(step: Step) {
  const c = step.confirmations;
  switch (step.status) {
    case "pending":
      return "Not yet";
    case "waiting":
      return "Waiting";
    case "seen":
      return c ? `Seen, waiting for confirmations (${c.have} of ${c.need})` : "Seen";
    case "confirmed":
      return c ? `Confirmed (${c.have} of ${c.need})` : "Done";
    case "failed":
      return "Didn't happen";
    case "skipped":
      return "Skipped";
  }
}

/** UX_SPEC §3: what happened, whether the funds are safe, what to do next. */
function errorCopy(code: ErrorCode, view: SwapView) {
  const t1 = when(view.refund_window.opens_at).local;
  const t2 = when(view.refund_window.closes_at).local;
  switch (code) {
    case "MAKER_NO_LOCK":
      return "The maker didn't lock BTC in time. Nothing left your wallet. You can close this.";
    case "MAKER_SILENT_AFTER_ZEC":
      return `The maker stopped responding. Your ZEC is safe and comes back after ${t1}. Keep the engine running, or come back before ${t2}.`;
    case "MARGIN_TOO_SMALL":
      return "It's too close to the deadline to claim BTC safely, so we're returning your ZEC instead. Nothing for you to do yet.";
    case "WRONG_ZEC_AMOUNT":
      return `The maker received a different amount than ${coin(view.zec_amount)} ZEC. This swap will be cancelled and your ZEC returned after ${t1}.`;
    case "BTC_SOURCES_DISAGREE":
      return "Two Bitcoin data sources disagree about the maker's lock. We won't show the payment QR until they agree. Your funds are safe.";
  }
}

function Qr({ text }: { text: string }) {
  const [svg, setSvg] = useState("");
  useEffect(() => {
    void QRCode.toString(text, { type: "svg", margin: 1, errorCorrectionLevel: "M", color: { dark: "#201613", light: "#ffffff" } }).then(setSvg);
  }, [text]);
  return <div className="qr" role="img" aria-label="Payment QR code" dangerouslySetInnerHTML={{ __html: svg }} />;
}

function Recovery({ engine, view }: { engine: Engine; view: SwapView }) {
  const [file, setFile] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let alive = true;
    const poll = async () => {
      const f = await engine.recoveryFile(view.swap_id).catch(() => null);
      if (!alive) return;
      if (f) setFile(f);
      else setTimeout(poll, 2000);
    };
    void poll();
    return () => {
      alive = false;
    };
  }, [engine, view.swap_id]);

  const download = () => {
    if (!file) return;
    const url = URL.createObjectURL(new Blob([file], { type: "text/plain" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `zecsplit-${view.swap_id.slice(0, 8)}.recovery`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <section className="swap-panel gold" aria-labelledby="rec-h">
      <span className="mono-label">Before you pay</span>
      <h3 id="rec-h">Save your recovery file.</h3>
      <p>
        It&apos;s encrypted with your passphrase. With it, you can finish this swap from another device if this
        one fails. It&apos;s the only way to recover.
      </p>
      {file ? (
        <>
          <div className="row-actions">
            <button type="button" className="btn btn-dark btn-sm" onClick={download}>
              Download
            </button>
            <Copy text={file} label="Copy as text" />
          </div>
          <label className="check">
            <input type="checkbox" checked={saved} onChange={(e) => setSaved(e.target.checked)} />
            <span>I saved my recovery file somewhere safe.</span>
          </label>
          <button
            type="button"
            className="btn btn-dark"
            disabled={!saved}
            onClick={() => void engine.recoverySaved(view.swap_id)}
          >
            Show the payment QR <span className="arrow">→</span>
          </button>
        </>
      ) : (
        <p className="status-line" role="status">
          <i className="spinner" /> Preparing your recovery file. It exists once you and the maker have agreed the
          Bitcoin lock.
        </p>
      )}
    </section>
  );
}

function Pay({ engine, view }: { engine: Engine; view: SwapView }) {
  const uri = view.zip321_uri ?? "";
  const deadline = view.steps.find((s) => s.kind === "pay_zec")?.deadline;
  return (
    <section className="swap-panel pay" aria-labelledby="pay-h">
      <div className="pay-qr">
        <Qr text={uri} />
      </div>
      <div className="pay-body">
        <span className="mono-label">Pay from your Zcash wallet</span>
        <h3 id="pay-h">
          {coin(view.zec_amount)} <small>ZEC</small>
        </h3>
        <p>
          Scan with a shielded wallet such as Zodl, Zkool or Vizor, or paste the address below. Send exactly this
          amount.
          {deadline && (
            <>
              {" "}
              Pay before <At unix={deadline.estimated_at} />.
            </>
          )}
        </p>
        <div className="uri">
          <code>{short(uri, 22)}</code>
          <Copy text={uri} />
        </div>
        {engine.simulatePayment && (
          <button type="button" className="btn btn-line btn-sm" onClick={() => engine.simulatePayment?.(view.swap_id)}>
            Simulate the payment (demo)
          </button>
        )}
      </div>
    </section>
  );
}

function Receipt({ view }: { view: SwapView }) {
  const claim = view.steps.find((s) => s.kind === "claim_btc");
  const link = claim?.txid ? explorer(view.network, claim.txid) : null;
  if (view.outcome === "redeemed") {
    return (
      <section className="swap-panel done" aria-labelledby="done-h">
        <span className="mono-label">
          <i className="led led-good" /> Swap complete
        </span>
        <h3 id="done-h">
          You received {coin(view.btc_amount)} BTC for {coin(view.zec_amount)} ZEC.
        </h3>
        {claim?.txid && (
          <div className="uri">
            <code>{short(claim.txid, 12)}</code>
            <Copy text={claim.txid} label="Copy txid" />
            {link && (
              <a className="mini-btn" href={link} target="_blank" rel="noreferrer">
                Explorer ↗
              </a>
            )}
          </div>
        )}
        <ul className="privacy-sum">
          <li>
            Your ZEC went from your wallet to a shielded two-key address, then to the maker. No transparent
            address was used.
          </li>
          <li>On Bitcoin, the amount and timing are public, as with any Bitcoin payment.</li>
        </ul>
      </section>
    );
  }
  const copy = {
    refunded: ["Your ZEC came back.", "The maker refunded on Bitcoin, which revealed their half of the key, and your engine swept your ZEC home."],
    punished: ["You received the maker's BTC as a penalty.", "The maker didn't refund before the second deadline, so your engine claimed the BTC."],
    aborted: ["The swap stopped before anything was at risk.", "Nothing left your wallet. You can start a new swap."],
  } as const;
  const [title, body] = copy[view.outcome as keyof typeof copy];
  return (
    <section className="swap-panel done" aria-labelledby="done-h">
      <span className="mono-label">Swap finished</span>
      <h3 id="done-h">{title}</h3>
      <p>{body}</p>
    </section>
  );
}

function Stepper({ view }: { view: SwapView }) {
  return (
    <ol className="stepper">
      {view.steps.map((s, i) => {
        const [title, help] = STEP_TEXT[s.kind];
        const link = s.txid ? explorer(view.network, s.txid) : null;
        return (
          <li key={s.kind} className={`st st-${s.status}`}>
            <span className="st-dot" aria-hidden="true">
              {s.status === "confirmed" ? "✓" : s.status === "failed" ? "✕" : String(i + 1).padStart(2, "0")}
            </span>
            <div className="st-body">
              <div className="st-head">
                <h4>{title}</h4>
                <span className="st-status" role="status">
                  {statusText(s)}
                </span>
              </div>
              <p>{help}</p>
              {s.txid && (
                <div className="uri small">
                  <code>{short(s.txid)}</code>
                  <Copy text={s.txid} />
                  {link && (
                    <a className="mini-btn" href={link} target="_blank" rel="noreferrer">
                      Explorer ↗
                    </a>
                  )}
                </div>
              )}
              {s.deadline && s.status !== "confirmed" && (
                <p className="st-deadline">
                  Deadline: block {s.deadline.height.toLocaleString()}, about <At unix={s.deadline.estimated_at} />
                </p>
              )}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

/** UX_SPEC §2.5: one screen for the whole swap, driven by the engine's view model. */
export function SwapProgress({ engine, view }: { engine: Engine; view: SwapView }) {
  // UX_SPEC §1.1: say so when nothing needs the user, and only then.
  const idle = !view.outcome && !view.error && view.recovery_saved && !view.zip321_uri;
  return (
    <div className="progress">
      <div className="swap-summary">
        <div>
          <span className="mono-label">You send</span>
          <strong>{coin(view.zec_amount)} ZEC</strong>
        </div>
        <span className="arrow-big" aria-hidden="true">
          →
        </span>
        <div>
          <span className="mono-label">You get</span>
          <strong>{coin(view.btc_amount)} BTC</strong>
        </div>
        <div className="sum-addr">
          <span className="mono-label">To</span>
          <code title={view.btc_address}>{short(view.btc_address, 10)}</code>
        </div>
      </div>

      {view.error && (
        <div className="swap-alert" role="alert">
          <strong>Heads up.</strong> {errorCopy(view.error, view)}
        </div>
      )}
      {view.outcome && <Receipt view={view} />}
      {!view.outcome && !view.recovery_saved && <Recovery engine={engine} view={view} />}
      {!view.outcome && view.zip321_uri && <Pay engine={engine} view={view} />}
      {idle && (
        <p className="status-line calm" role="status">
          <i className="spinner" /> Nothing for you to do right now. You can leave this open; the engine keeps
          working in the background.
        </p>
      )}

      <Stepper view={view} />

      <p className="window-note">
        If the maker disappears, your ZEC comes back automatically, as long as the engine runs at some point
        between <At unix={view.refund_window.opens_at} /> and <At unix={view.refund_window.closes_at} />.
      </p>
    </div>
  );
}
