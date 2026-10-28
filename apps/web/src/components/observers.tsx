"use client";

import { useState } from "react";

/** PRIVACY.md §1, in plain words. */
const OBSERVERS = [
  {
    name: "The Zcash chain",
    learns: [
      "Two shielded transactions: your payment into the swap address, and the maker's sweep out of it",
      "Their fees",
    ],
    never: ["Any amount", "Any address", "That the two transactions are related", "Anything about Bitcoin"],
  },
  {
    name: "The Bitcoin chain",
    learns: [
      "One 2-of-2 script output and the transaction that spends it",
      "The BTC amount and your payout address, like any Bitcoin payment",
    ],
    never: ["That this was a Zcash swap", "The ZEC amount", "Your Zcash wallet"],
  },
  {
    name: "The maker",
    learns: ["Both amounts and the timing", "Your BTC payout address", "Your IP address (Tor support is planned)"],
    never: ["Your Zcash wallet, balance or other notes", "Anything that lets them take your coins"],
  },
  {
    name: "A network observer",
    learns: [
      "That you're talking to a maker",
      "Over plain HTTP, the swap messages too: reach the maker over HTTPS or a tunnel (Tor support is planned)",
    ],
    never: ["Anything that lets them take your coins"],
  },
];

export function Observers() {
  const [active, setActive] = useState(0);
  const o = OBSERVERS[active];

  return (
    <div className="observers reveal">
      <div className="observer-tabs" role="tablist" aria-label="Who learns what">
        {OBSERVERS.map((ob, i) => (
          <button
            key={ob.name}
            type="button"
            role="tab"
            id={`obs-tab-${i}`}
            aria-selected={i === active}
            aria-controls="obs-panel"
            className="observer-tab"
            onClick={() => setActive(i)}
          >
            <span>{ob.name}</span>
            <span aria-hidden="true">{i === active ? "→" : ""}</span>
          </button>
        ))}
      </div>
      <div className="observer-panel" role="tabpanel" id="obs-panel" aria-labelledby={`obs-tab-${active}`} key={active}>
        <div className="observer-col">
          <span className="label">Learns</span>
          <ul>
            {o.learns.map((l) => (
              <li key={l}>
                <span className="mark-learn" aria-hidden="true">+</span>
                {l}
              </li>
            ))}
          </ul>
        </div>
        <div className="observer-col">
          <span className="label">Never learns</span>
          <ul>
            {o.never.map((l) => (
              <li key={l}>
                <span className="mark-hidden" aria-hidden="true">■</span>
                {l}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}
