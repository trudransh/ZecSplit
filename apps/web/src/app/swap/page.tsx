import type { Metadata } from "next";
import { Nav } from "@/components/nav";
import { SiteFooter } from "@/components/site-footer";
import { SwapApp } from "@/components/swap/swap-app";

export const metadata: Metadata = {
  title: "Swap · ZecSplit",
  description: "Swap shielded ZEC for BTC with your own local engine. No bridge, no custodian, no transparent address.",
};

const DOCS = "https://zecsplit.gitbook.io/zecsplit-docs";

export default function SwapPage() {
  return (
    <>
      <Nav docs={DOCS} />
      <main className="wrap swap-page">
        <header className="swap-head">
          <span className="mono-label">
            <i className="sq sq-zec" /> Shielded ZEC → BTC
          </span>
          <h1>
            Swap shielded ZEC
            <br />
            <span className="mute">for Bitcoin.</span>
          </h1>
        </header>
        <SwapApp />
      </main>
      <SiteFooter />
    </>
  );
}
