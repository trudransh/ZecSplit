# FAQ

Short answers to common questions.

## Can I use it with real money?

No. ZecSplit is pre-alpha and unaudited. It runs on regtest and testnet only.

## Do I need a special Zcash wallet?

No. You pay a standard ZIP 321 request from the shielded wallet you already use. ZecSplit never holds your ZEC.

## Can the maker run off with my ZEC?

No. The ZEC sits under a key that needs both shares. Bob only learns your share when you claim his BTC on chain, and by then the BTC is yours.

## What if the maker disappears after I pay?

After `t1` blocks your engine cancels. If Bob refunds, his refund reveals his share and your engine sweeps your ZEC back. If he never refunds, after `t2` more blocks you take his BTC instead.

## What if my computer is off during the swap?

The engine saves every step before acting and resumes on start. You need to come back online before the deadlines if something goes wrong; the UI shows them. A watchtower that removes this requirement is planned.

## Does anything touch a transparent Zcash address?

No. Your payment into the lock and the sweep out are both shielded Ironwood transactions.

## How long does a swap take?

A few Bitcoin blocks on the happy path. If something goes wrong, up to `t1 + t2` blocks (about 12 hours with the testnet settings).

## Why only ZEC → BTC?

It is the exit shielded holders lack. BTC → ZEC uses the same protocol with roles swapped and is planned next.

## Is this a bridge or a DEX?

Neither. There is no wrapped asset, no pool and no order book. Two people trade directly, and the protocol makes the trade all-or-nothing.
