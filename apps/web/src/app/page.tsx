import Link from "next/link";
import { HeroArt, Ruler } from "@/components/hero-art";
import { BitcoinMark, Logo, ZcashMark } from "@/components/logo";
import { Nav } from "@/components/nav";
import { Observers } from "@/components/observers";
import { Ocean } from "@/components/ocean";
import { RouteScene } from "@/components/route-scene";

const REPO = "https://github.com/Raveesh1007/Splitzec";
const DOCS = "https://zecsplit.gitbook.io/zecsplit-docs";

const ROUTES: { n: string; tab: string; scene: "bridge" | "exchange" | "pool"; lead: string[]; body: string; chips: string[] }[] = [
  {
    n: "01",
    tab: "A bridge or an intent.",
    scene: "bridge",
    lead: ["Fast and cheap.", "Visible, and freezable."],
    body: "Your ZEC goes to a fresh transparent deposit address first. A solver or a compliance desk can hold it there.",
    chips: ["NEAR Intents", "Wallet swaps"],
  },
  {
    n: "02",
    tab: "An exchange.",
    scene: "exchange",
    lead: ["Liquid.", "Yours until it isn't."],
    body: "Know-your-customer checks, deposit addresses and withdrawal freezes. Delistings come without notice.",
    chips: ["KYC", "Custody"],
  },
  {
    n: "03",
    tab: "A cross-chain pool.",
    scene: "pool",
    lead: ["Decentralised.", "Still transparent."],
    body: "THORChain and Maya hold ZEC in a transparent vault, so every swap through them can be traced.",
    chips: ["THORChain", "Maya"],
  },
];

const ROWS: [string, string, string][] = [
  ["sender", "your shielded wallet", ""],
  ["deposit address", "t1Kx9fRw…Q2mW", "none"],
  ["amount", "12.50000000 ZEC", ""],
  ["link to Bitcoin", "traceable", "none"],
  ["can be frozen by", "solver, compliance", "nobody"],
];

const STEPS = [
  ["Get a quote", "Enter how much ZEC to swap. The maker signs a quote that's valid for 60 seconds."],
  ["Save your recovery file", "Encrypted with your passphrase. With it, you can finish the swap from any device."],
  ["The maker locks BTC first", "You pay nothing until the maker's BTC is locked and confirmed on Bitcoin."],
  ["Pay one QR code", "From the Zcash wallet you already use: Zodl, Zkool or Vizor. The payment stays shielded."],
  ["Claim your BTC", "The app claims it to your own Bitcoin address. Then the maker sweeps the ZEC."],
];

const FAILURES = [
  ["Maker never locks", "You never pay. There's nothing to undo, and both keep their coins."],
  ["You never pay", "After the first deadline, the maker takes their BTC back."],
  ["Maker goes silent", "The app cancels. The maker's refund reveals their half of the key, and your ZEC is swept back to you."],
  ["Maker never refunds", "After the second deadline, the app claims the maker's BTC as a penalty."],
];

const ZCASH: { tag: string; title: string; body: string; scene: "notes" | "split" | "adaptor" }[] = [
  {
    tag: "Shielded notes",
    title: "Encrypted on chain.",
    body: "Validators check a zero-knowledge proof that a payment is valid without seeing the sender, the receiver or the amount.",
    scene: "notes",
  },
  {
    tag: "Split key",
    title: "Half a key is no key.",
    body: "Your ZEC sits at a fresh shielded address whose key is two secret halves added together. You hold one, the maker holds the other.",
    scene: "split",
  },
  {
    tag: "Adaptor signatures",
    title: "Claiming hands over one half.",
    body: "Your Bitcoin claim reveals your half of the key, and only then can the maker sweep the ZEC. One action settles both sides.",
    scene: "adaptor",
  },
];

const LIMITS = [
  ["The maker sees the deal", "Both amounts, the timing and your BTC payout address. Every quote needs them."],
  ["The maker sees your IP", "Tor support is planned. Until then, use your own VPN or Tor if that matters to you."],
  ["Bitcoin is public", "The BTC amount and your payout address are on Bitcoin, like any Bitcoin payment."],
  ["Timing can hint", "Someone watching both chains could guess from timing. You can wait before you pay."],
];

function Chip({ children }: { children: React.ReactNode }) {
  return <span className="chip">{children}</span>;
}

export default function Home() {
  return (
    <>
      <Nav docs={DOCS} />

      <main>
        <section className="hero wrap">
          <div className="hero-copy">
            <h1>
              Shielded ZEC in.
              <br />
              Bitcoin out.
            </h1>
            <p className="lede">
              A trustless atomic swap from shielded ZEC to BTC. No bridge, no custodian and no transparent address,
              so nobody in the middle can freeze it, take it or watch it.
            </p>
            <div className="actions">
              <Link href="/swap" className="btn btn-dark">
                Start swap <span className="arrow">→</span>
              </Link>
              <a href="#how" className="btn btn-line">
                See how it works
              </a>
            </div>
          </div>
          <HeroArt />
          <Ruler />
        </section>

        <section className="strip">
          <div className="wrap strip-inner">
            <div>
              <span className="mono-label">Built on</span>
              <div className="logo-row">
                <span className="logo-box">
                  <ZcashMark /> Zcash
                </span>
                <span className="logo-box">
                  <BitcoinMark /> Bitcoin
                </span>
                <span className="logo-box">
                  <span className="mono">Gugger · 2020</span>
                </span>
              </div>
            </div>
            <div className="strip-side">
              <span className="mono-label">Built for</span>
              <div className="logo-row">
                <span className="logo-box">
                  <span className="mono">Zecathon · 2026</span>
                </span>
              </div>
            </div>
          </div>
        </section>

        <section className="section wrap">
          <h2>
            Every way out of shielded ZEC
            <br />
            <span className="mute">has someone in the middle.</span>
          </h2>
          <div className="folders">
            {ROUTES.map((r) => (
              <article className="folder" key={r.n}>
                <div className="folder-art">
                  <span className="folder-tab">
                    <span className="num">{r.n}</span> {r.tab}
                  </span>
                  <RouteScene name={r.scene} />
                </div>
                <div className="folder-body">
                  <h3>
                    {r.lead[0]}
                    <br />
                    {r.lead[1]}
                  </h3>
                  <p>{r.body}</p>
                  <div className="chips">
                    {r.chips.map((c) => (
                      <Chip key={c}>{c}</Chip>
                    ))}
                  </div>
                </div>
              </article>
            ))}
          </div>
        </section>

        <section className="section sand" id="zcash">
          <div className="wrap">
            <h2 className="center">
              Three things only
              <br />
              Zcash makes possible.
            </h2>
            <div className="three">
              {ZCASH.map((z) => (
                <article className="three-card" key={z.tag}>
                  <span className="mono-label">
                    <i className="sq sq-zec" /> {z.tag}
                  </span>
                  <div className="three-art">
                    <RouteScene name={z.scene} variant="compact" />
                    <span className="three-title">{z.title}</span>
                  </div>
                  <p>{z.body}</p>
                </article>
              ))}
            </div>
          </div>
        </section>

        <section className="section wrap" id="safety">
          <div className="dark-folder">
            <span className="dark-tab">Safety &amp; recovery</span>
            <div className="dark-grid">
              <div>
                <h2>
                  If anything goes wrong,
                  <br />
                  <span className="mute">Bitcoin&apos;s clock brings your coins home.</span>
                </h2>
                <p className="lede">
                  Every deadline lives in one Bitcoin script. Nobody has to cooperate for your money to come back.
                  The app only has to run at some point inside the refund window.
                </p>
                <a href={DOCS} target="_blank" rel="noreferrer" className="btn btn-cyan">
                  Read the protocol <span className="arrow">→</span>
                </a>
              </div>
              <div className="safety-stage">
                <RouteScene name="safety" variant="full" steps={FAILURES.map(([title]) => title)} />
              </div>
            </div>
          </div>
        </section>

        <section className="section wrap" id="privacy">
          <h2 className="narrow">
            A bridge shows everyone what you swapped. <span className="mute">ZecSplit shows no one.</span>
          </h2>
          <p className="lede">
            On Zcash, a ZecSplit swap is two ordinary shielded transactions. Nothing on chain ties them to each other
            or to the Bitcoin side.
          </p>
          <div className="versus">
            <div className="panel panel-grey">
              <div className="panel-tags">
                <Chip>Bridge route</Chip>
                <Chip>
                  <i className="led led-bad" /> Transparent
                </Chip>
              </div>
              <h3>Your swap on a block explorer.</h3>
              <p className="panel-sub">Public the moment it&apos;s mined, and forever after.</p>
              <div className="card">
                <div className="card-head">
                  Visible on chain: <span className="score score-bad">5 of 5</span>
                </div>
                {ROWS.map(([k, v]) => (
                  <div className="row" key={k}>
                    <span className="pill">{k}</span>
                    <span className="val leak">{v}</span>
                  </div>
                ))}
              </div>
            </div>
            <div className="panel panel-gold">
              <div className="panel-tags">
                <Chip>
                  <Logo size={14} /> ZecSplit
                </Chip>
                <Chip>
                  <i className="led led-good" /> Shielded
                </Chip>
              </div>
              <h3>The same swap through ZecSplit.</h3>
              <p className="panel-sub">Two shielded transactions. Nothing to read.</p>
              <div className="card">
                <div className="card-head">
                  Visible on chain: <span className="score score-good">0 of 5</span>
                </div>
                {ROWS.map(([k, , ours]) => (
                  <div className="row" key={k}>
                    <span className="pill">{k}</span>
                    {ours ? <span className="val">{ours}</span> : <span className="redact" aria-label="hidden" />}
                  </div>
                ))}
              </div>
            </div>
          </div>

          <div className="halftone">
            <div className="window">
              <div className="crumb">
                <span>Privacy</span> <span aria-hidden="true">›</span> <span>Who learns what</span>
              </div>
              <Observers />
            </div>
          </div>
          <p className="statement">
            Every claim on this page comes from our published privacy analysis, including the leaks we can&apos;t
            close yet. They&apos;re listed at the bottom.
          </p>
        </section>

        <section className="section wrap split" id="how">
          <div>
            <h2>
              <span className="mute">About an hour.</span>
              <br />
              One QR code.
            </h2>
            <p className="lede">
              A swap takes about 45 to 90 minutes, mostly waiting for confirmations. You act twice: once to start, and
              once to pay.
            </p>
          </div>
          <ol className="rail">
            {STEPS.map(([title, body], i) => (
              <li key={title}>
                <span className="rail-n">STEP {String(i + 1).padStart(2, "0")}</span>
                <div>
                  <h3>{title}</h3>
                  <p>{body}</p>
                </div>
              </li>
            ))}
          </ol>
        </section>

        <section className="section wrap" id="limits">
          <h2 className="narrow">
            What ZecSplit doesn&apos;t hide.
            <br />
            <span className="mute">Exact edges build trust.</span>
          </h2>
          <div className="limits">
            {LIMITS.map(([title, body]) => (
              <div className="limit" key={title}>
                <h3>{title}</h3>
                <p>{body}</p>
              </div>
            ))}
          </div>
          <p className="mono-note">
            Pre-alpha research software · Not audited · Test networks only · Don&apos;t use real funds
          </p>
        </section>
      </main>

      <footer className="footer">
        <Ocean className="ocean" />
        <div className="ocean-bar">
          <span>© 2026 ZecSplit · MIT or Apache-2.0</span>
          <nav aria-label="Footer">
            <a href="#how">How it works</a>
            <a href="#privacy">Privacy</a>
            <a href={DOCS} target="_blank" rel="noreferrer">
              Docs
            </a>
            <a href={REPO} target="_blank" rel="noreferrer">
              GitHub
            </a>
          </nav>
          <span>Testnet only · Unaudited</span>
        </div>
      </footer>
    </>
  );
}
