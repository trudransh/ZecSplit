# Paper

ZecSplit, trustless atomic swaps between shielded Zcash and Bitcoin. Draft v0.1.

*Draft v0.1. The evaluation section is measured on a live swap on Zcash testnet and Bitcoin signet.*

## Abstract

Shielded Zcash has no scripting language. That has kept cross-chain trades with shielded ZEC dependent on intermediaries: bridges, custodians and intent solvers. These can freeze funds, and they force a transparent hop.

We present ZecSplit, a non-custodial atomic swap between Zcash Ironwood notes and Bitcoin. It adapts the Bitcoin–Monero protocol of Gugger (2020):

* The ZEC is locked to an Ironwood address whose spend-authorizing key is the sum of two secret shares.
* Every conditional rule (timeouts, refund, penalty) lives in Bitcoin script.
* Each Bitcoin spending path reveals exactly one share to the counterparty, through ECDSA adaptor signatures.
* A cross-group discrete-log equality proof ties each Pallas share to its secp256k1 twin.

On Zcash, a swap is two ordinary shielded transactions with no transparent component.

## 1. Contributions

1. A protocol for atomic swaps between shielded Zcash (Ironwood) and Bitcoin with no Zcash consensus change. It adapts Gugger's construction to Orchard-style keys, including the `ỹ(ak) = 0` sign rule and viewing keys derived from the transcript.
2. A security analysis covering rogue-key attacks, the two Bitcoin race windows, and lying data sources on both chains.
3. A privacy comparison with intent-based exits.
4. An open-source implementation with an adversarial test suite, a model check of both state machines, and a live swap on Zcash testnet and Bitcoin signet.

## 2. Background

**Ironwood / Orchard keys.** The full viewing key is `(ak, nk, rivk)`, with `ak = [ask]·SpendAuthG` on Pallas. Spending needs a RedPallas signature under the randomized key `rk = ak + [α]·SpendAuthG`, made with `rsk = ask + α`. The zero-knowledge proof needs only viewing-level data, never `ask`. So `ask` can be shared additively, and the signature applied externally (the `pczt` Signer role).

**Bitcoin.** Relative timelocks (BIP68) on pre-signed transactions, 2-of-2 P2WSH outputs, BIP143 signature hashes.

**Adaptor signatures.** An encrypted ECDSA signature under a point `Y = [y]·G` can be decrypted with `y` into a valid signature. Seeing both the encrypted and the decrypted signature reveals `y`.

**Cross-group DLEq.** A proof that points in two groups of different order share a discrete log, built from bit-decomposition ring proofs. The implementation is Serai's `dleq`.

**Gugger's swap.** The chain without scripts holds funds under a key `a + b`. The scripted chain holds the other asset with cancel, refund, punish and redeem paths that reveal `a` or `b`.

## 3. Protocol

See [Protocol](../protocol.md) for the normative version. In summary:

1. **Setup.** Commit-reveal of the key shares, with DLEq and proofs of knowledge. The shared full viewing key is derived from the transcript. Four Bitcoin transactions are pre-signed; Alice's refund signature is encrypted under Bob's adaptor point.
2. **Lock.** Bob locks BTC. Alice pays ZEC to the shared Ironwood address from any wallet, using a ZIP 321 request.
3. **Swap.** Bob sends a redeem signature encrypted under Alice's point. Alice decrypts it and claims the BTC, which reveals `s_a`. Bob sweeps the ZEC with `σ·(s_a + s_b)`.
4. **Recovery.** After `t1`: cancel, then Bob refunds (revealing `s_b`) and Alice sweeps her ZEC back. After `t2` with no refund: Alice punishes and takes the BTC.

## 4. Security (informal)

**Atomicity.** In every terminal state, each party holds either its original asset or the counterparty's:

| Ending | Outcome                                                                     |
| ------ | --------------------------------------------------------------------------- |
| Redeem | Alice has BTC; Bob has `s_a + s_b`, so the ZEC                              |
| Refund | Bob has BTC; Alice has `s_a + s_b`, so the ZEC                              |
| Punish | Alice has BTC; the ZEC is unspendable. Bob is penalized for staying silent. |

**Key generation.** Commit-reveal plus proofs of knowledge stop either party from choosing a share as a function of the other's.

**Races.** Revealing `s_a` too close to `t1` is unsafe, so we require a margin. Revealing `s_b` too close to `t2` is unsafe, so we require prompt refunds and CPFP.

**Data sources.** Bob verifies the ZEC lock on his own node. Alice requires two independent Bitcoin sources before paying.

**Mechanical checks.** Both state machines are pure functions, so a bounded model check drives each through 20,000 random runs of 60 steps over adversarial chain views (reorgs, disagreeing sources, jumps) and peer inputs, checking the funds-safety rules after every step. It found one bug before release: a reorg of the BTC lock could let a taker who had already paid abort the swap.

## 5. Privacy

Zcash sees two ordinary Ironwood transactions. Bitcoin sees an ordinary 2-of-2 spend. The counterparty learns both amounts. Optional Tor hides IPs. See [Privacy](../security/privacy.md).

## 6. Implementation

Rust crates for the cryptography, Zcash, Bitcoin and the protocol; a local engine (`zecsplitd`); a maker service; a web UI. Built on pinned pre-release `librustzcash` crates for NU7, the `orchard` and `pczt` crates, Serai's `dleq` and `ecdsa_fun`. See [Developers](../developers.md).

## 7. Evaluation

Measured on swap `550dc7bd…` (Zcash testnet + Bitcoin signet, 26 Oct 2026): maker on a 4-vCPU cloud VM with its own Zebra node, taker engine on a laptop, both release builds.

| Metric                                                             | Value                    |
| ------------------------------------------------------------------ | ------------------------ |
| Setup (commit-reveal, DLEq proofs, pre-signed transactions)        | 8 s                      |
| BTC lock confirmed (1 signet confirmation)                         | 5.8 min after the quote  |
| ZEC lock to the maker's redeem signature (5 confirmations at 25 s) | \~4 min                  |
| Maker side done (redeem seen → ZEC sweep broadcast → confirmed)    | 13.5 min after the quote |
| Zcash sweep build and Ironwood proof (maker VM)                    | 13 s                     |
| End to end, taker side (until the redeem confirms)                 | 35.9 min                 |
| Bitcoin fees at 2 sat/vB: lock (maker's wallet) / redeem           | 153 / 276 sats           |
| Bitcoin fees, refund path and punish path                          | \~2 × 280 sats each      |
| Adversarial tests passing                                          | 10 / 14                  |
| Scenario tests on regtest through the real engine and maker        | S-01 to S-07             |

Transactions: lock [`dbc65938…`](https://mempool.space/signet/tx/dbc6593847db04dec256aca4dae00c8b5df6a478731d981291be2c7b7937cda1), ZEC payment [`62cfea06…`](https://testnet.zecblock.com/tx/62cfea0652d9ec5675db266bcc28f6bac7e63b0ba0f2443b5a1cf03e1e88805c), redeem [`8968f8f0…`](https://mempool.space/signet/tx/8968f8f08afe3f0bce599b47115f28f42bd0d67ea04f1a92df9137fa525be35d), sweep [`22451c4e…`](https://testnet.zecblock.com/tx/22451c4e6e6c8e720ddfec7cc45756b76ed3e9cc95628773c08c3685e94a25e7).

## 8. Related work

| Work                                           | What it does                                     | Difference                                                                      |
| ---------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------- |
| Gugger 2020; COMIT `xmr-btc-swap`; eigenwallet | BTC ↔ XMR, in production since 2021              | We port it to Zcash Ironwood keys                                               |
| ASMR (2021)                                    | ZEC Sapling ↔ BTC proof of concept, unmaintained | Ironwood, PCZT, NU7                                                             |
| "Zwap" design (Zcash forum 44864, 2023)        | BTC/ZEC design with P2TR, never built            | A built implementation                                                          |
| Adaptor signature on the Zcash side            | Proposed designs                                 | Open to front-running (forum 57884); we keep the ZEC under a shared key instead |
| Intent networks                                | Solver network, transparent deposit              | Custody and compliance in the path                                              |

## 9. Limitations and future work

* Liveness requirement (a watchtower is planned).
* Liquidity depends on makers.
* Not quantum-recoverable while locked.
* Taproot with Schnorr adaptors would improve Bitcoin-side privacy.
* An EVM counter-chain through a scalar-reveal contract.
* Chain verification without trusting a server.

## References

* J. Gugger, "Bitcoin–Monero Cross-chain Atomic Swap", 2020: <https://github.com/h4sh3d/xmr-btc-atomic-swap>
* COMIT, xmr-btc-swap: <https://github.com/comit-network/xmr-btc-swap> ; eigenwallet: <https://github.com/eigenwallet/core>
* Serai `dleq`: <https://crates.io/crates/dleq> ; `ecdsa_fun`: <https://crates.io/crates/ecdsa\\_fun>
* Zcash protocol specification and ZIPs 32, 203, 244, 316, 317, 321, 324, 2005
