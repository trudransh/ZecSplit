<p align="center">
  <img src="docs/assets/logo.svg" width="96" alt="ZecSplit">
</p>

<h1 align="center">ZecSplit</h1>

<p align="center">
  <b>Swap shielded ZEC for BTC with nobody in the middle who can freeze or steal it.</b>
</p>

<p align="center">
  <a href="https://github.com/trudransh/Splitzec/actions/workflows/ci.yml"><img src="https://github.com/trudransh/Splitzec/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License">
  <img src="https://img.shields.io/badge/status-pre--alpha-orange" alt="Status: pre-alpha">
  <img src="https://img.shields.io/badge/networks-regtest%20%7C%20testnet-lightgrey" alt="Networks">
  <a href="docs/README.md"><img src="https://img.shields.io/badge/docs-read-blue" alt="Docs"></a>
</p>

> [!WARNING]
> **Pre-alpha. Regtest and testnet only. Unaudited.** Do not use with real funds.

## For judges

1. **Proof on public chains.** Four swaps and one timelock refund on Zcash testnet + Bitcoin signet, every transaction linked in [Live on testnet](#live-on-testnet). The fourth swap ran through the hosted web UI.
2. **The UI.** [zecsplit.vercel.app](https://zecsplit.vercel.app). The page is only a screen: a real swap needs the engine running on your own computer, because that is where the keys live. *Try the demo* is a simulation with no chain behind it.
3. **Run a real swap yourself, no coins needed.** With Docker and Rust, `just judge` starts a private regtest world on your machine (Zebra, bitcoind, a maker and your engine) and prints a link that opens the hosted UI against it. See [Judge kit](#judge-kit).
4. **Privacy, checked on real chains.** Every Zcash transaction a swap makes has 0 transparent parts, 0 Sapling parts and exactly 2 Ironwood actions; no secret appears in any file or log the engine or maker writes. `just demo` runs these checks.

The name means a *split key*: the ZEC sits under a spend key split between the two parties. Nothing to do with splitting bills.

## What it is

ZecSplit is a trustless atomic swap between **shielded ZEC** (Zcash Ironwood pool) and **BTC**. No bridge, no custodian, no wrapped coin and no transparent Zcash address at any point.

Shielded Zcash has no scripting language, so it can't hold a hash-locked contract. ZecSplit adapts the Bitcoin–Monero protocol of Gugger (2020) instead:

- The ZEC is locked to a fresh shielded address whose spending key is `ask = σ·(s_a + s_b)`, the sum of two secret shares. Alice holds `s_a`, Bob holds `s_b`, and neither can spend alone.
- Every condition (redeem, cancel after `t1`, refund, punish after `t2`) lives in **Bitcoin script**.
- Each Bitcoin spending path reveals **exactly one share** to the other party through an ECDSA adaptor signature. Whoever learns the missing share can sweep the ZEC.
- A cross-group discrete-log equality proof ties each share on Pallas (Zcash) to its twin on secp256k1 (Bitcoin), so neither side can cheat with mismatched keys.

On Zcash, a swap is two ordinary shielded transactions. Nothing on the Zcash chain links them to the Bitcoin side.

## How a swap works

Alice has ZEC and wants BTC (the taker). Bob has BTC and wants ZEC (the maker).

```mermaid
sequenceDiagram
    autonumber
    participant A as Alice (taker engine)
    participant B as Bob (maker)
    participant BTC as Bitcoin
    participant ZEC as Zcash (shielded)

    A->>B: commit, then reveal: share pubkeys + DLEq proofs
    B->>A: commit, then reveal
    Note over A,B: both derive the lock address for ask = σ·(s_a + s_b)
    B->>A: tx_lock template
    A->>B: sig on tx_cancel + adaptor sig on tx_refund
    B->>A: sigs on tx_cancel + tx_punish
    B->>BTC: tx_lock (BTC into the 2-of-2 script)
    A->>ZEC: pay ZEC to the lock address
    B->>A: adaptor sig on tx_redeem (once ZEC is confirmed)
    A->>BTC: tx_redeem: Alice gets the BTC, s_a is revealed
    B->>ZEC: Bob reads s_a from tx_redeem and sweeps the ZEC
```

If something goes wrong, the timelocks decide:

| Situation | What happens | Who gets what |
|---|---|---|
| Bob never locks BTC | Alice never pays ZEC. Nothing to undo. | Both keep their coins |
| Alice never pays ZEC | After `t1`, `tx_cancel`, then Bob's `tx_refund` | Bob gets his BTC back |
| Bob goes silent after Alice pays | After `t1`, Alice cancels. Bob's `tx_refund` reveals `s_b` | Bob gets BTC back, Alice sweeps her ZEC back |
| Bob never refunds | After `t2`, Alice's `tx_punish` | Alice takes the BTC; the ZEC stays locked |

## What a swap proves, and what it doesn't

| A completed swap proves | It does not prove |
|---|---|
| Neither party could take both coins | That the price was fair: the signed quote only proves the maker offered it |
| The ZEC never touched a transparent address | That the BTC side is private (Bitcoin is public) |
| Bitcoin facts were agreed by at least two independent sources | Anything about the code: it is unaudited |

## Status

Built for the Zecathon (Cross-Chain track).

| Piece | State |
|---|---|
| Cross-group DLEq, commit-reveal, proofs of knowledge | Done; rogue-key and mismatched-twin attacks rejected |
| Shared-key Ironwood lock and sweep | Done, on regtest and on Zcash testnet |
| Bitcoin script, 5 transactions, adaptor signatures | Done; redeem, cancel, refund and punish mined on regtest |
| Alice and Bob state machines, write-ahead store, recovery file | Done; both sides resume after a crash at any state |
| Engine (local HTTP API + SSE) and maker service | Done; separate processes over HTTP |
| Every scenario on regtest through the real engine and maker | Done: happy path, maker never locks, taker never pays, maker blind to ZEC, maker never refunds, overpayment, crash every tick |
| **Live swap on Zcash testnet + Bitcoin signet** | **Done** (txids below) |
| Web UI (landing + swap) | Done |

## Live on testnet

Shielded testnet ZEC swapped for signet BTC, the maker on a cloud VM checking the lock on its own Zebra node, the taker's engine on a laptop with two Bitcoin sources.

| Swap | Maker locks BTC (signet) | Taker pays ZEC (testnet) | Taker claims BTC (signet) | Maker sweeps ZEC (testnet) |
|---|---|---|---|---|
| 1 | [`dbc65938…`](https://mempool.space/signet/tx/dbc6593847db04dec256aca4dae00c8b5df6a478731d981291be2c7b7937cda1) | [`62cfea06…`](https://testnet.zcashexplorer.app/transactions/62cfea0652d9ec5675db266bcc28f6bac7e63b0ba0f2443b5a1cf03e1e88805c) | [`8968f8f0…`](https://mempool.space/signet/tx/8968f8f08afe3f0bce599b47115f28f42bd0d67ea04f1a92df9137fa525be35d) | [`22451c4e…`](https://testnet.zcashexplorer.app/transactions/22451c4e6e6c8e720ddfec7cc45756b76ed3e9cc95628773c08c3685e94a25e7) |
| 2 | [`224e049f…`](https://mempool.space/signet/tx/224e049f3ba6f7945304e0ed684f8009ccfb1159bf557bb4c7d51873e17af774) | [`2139bd7a…`](https://testnet.zcashexplorer.app/transactions/2139bd7a3a0cb4b24438bca3cf3464d6e83d5eea384fbcd1901114e5f0a8824d) | [`f1569274…`](https://mempool.space/signet/tx/f15692748402e1730c8d36fee1ff3d212057027ac40668381a249f640d2205bd) | [`9125fcb0…`](https://testnet.zcashexplorer.app/transactions/9125fcb06fa15dce4301c17bbbe3b451de0a1769c710d8b1cbaec1f8bfe6a8fc) |
| 3 | [`27024c5d…`](https://mempool.space/signet/tx/27024c5da837cb9ed1dc4eeda2c2f128d4692418e9dea07000f0bdee7af48f9a) | [`abdb747f…`](https://testnet.zcashexplorer.app/transactions/abdb747f0f471fd6a9844940c2db95ef43e68c5362a8a99912ed19f57e189aa6) | [`dce3fce1…`](https://mempool.space/signet/tx/dce3fce1aa550cab3b3e7e2b190b1a2b4388f51104471d375252fcfaf29b3d35) | [`85af11f5…`](https://testnet.zcashexplorer.app/transactions/85af11f506ee0fbf558b2f9d612f7bef1d05d68d027186bb8c02208d528a2c30) |
| 4 (web UI) | [`7e91162a…`](https://mempool.space/signet/tx/7e91162ab130a637f22cde5be9c86517f9b475d6be5473fa6892415ced5cfc8a) | [`e3585ecb…`](https://testnet.zcashexplorer.app/transactions/e3585ecb7f233cb4f3ef4a1d89fec486768330e66beaddb4d4427a91a437ca46) | [`e54c639f…`](https://mempool.space/signet/tx/e54c639f355efa075fa80319b465789fb2528c7ef383393822356a534f052f2b) | [`2e264fe9…`](https://testnet.zcashexplorer.app/transactions/2e264fe99a1864169301fc8dad0e950ac8550cc2f5630f01b8d08f09154168b7) |

**A refund on a public chain.** Swap `d9c9794f` stopped after the maker locked BTC and before any ZEC was paid. Nobody cooperated; the timelocks finished it:

| Maker locks BTC | Cancel, valid only from `t1` = 36 blocks after the lock | Maker's refund |
|---|---|---|
| [`d1dcc58c…`](https://mempool.space/signet/tx/d1dcc58c246451446b20f6a305f4e2697cbd0ad605d9e8a3ddfceb59fab3e5a8) at block 328056 | [`efed9957…`](https://mempool.space/signet/tx/efed99572f863eb1782088da7a8e1b2ebe17b0f3265fd970325a04f8dd28659a) at block 328092 | [`1503cf53…`](https://mempool.space/signet/tx/1503cf53e194fd1affae7f481f3f7b08057315daae8b84652dc96c2ae5a5ceab) at block 328093 |

How these ran, and what it means for trust:

- The maker ran on a cloud VM and checked the ZEC lock on its own Zebra node. The taker's engine ran on a laptop.
- The taker's two Bitcoin sources were the maker's bitcoind and the public mempool.space signet API. Both must agree before the taker acts, so a lying maker node can only stall a swap, never make the taker pay early.
- The taker's Zcash light server was the maker's. The taker only uses it on the refund path, so in a refund the maker would see the taker's sweep. A taker-owned light server replaces it after the hackathon.

The operator runbook is in [`ops/testnet`](ops/testnet); it expects your own maker VM and wallets. For a testnet session against our maker, open an issue.

## How it has been attacked

- **Model check.** Both state machines are driven through 20,000 random runs each, with reorgs, disagreeing Bitcoin sources, block jumps and hostile peer messages. After every step it checks the rules that keep funds safe: no payment request before a confirmed lock, no redeem inside the safety margin, no cancel against your own redeem, no sweep without the other side's secret, no abort after paying or locking. It found a real bug (a reorg could abort a taker who had already paid), now fixed.
- **Fuzzing.** Mutated setup signatures, adaptor signatures, lock templates and reveals are all rejected, before and after the real one, and never leave a session unusable. The wire and recovery-file decoders take 60,000 random inputs without a panic.
- **Adversarial tests** for a fake BTC lock, a maker checking ZEC on someone else's node, wrong ZEC amounts, a signature arriving inside the safety margin, a blocked refund (the taker punishes) and quote flooding.
- **Privacy tests on real chains.** Every Zcash transaction has zero transparent and zero Sapling parts and exactly 2 Ironwood actions; no secret appears in any file the engine or maker writes; nothing asks a light server for a transaction by id.

## Architecture

```mermaid
flowchart BT
    crypto[zecsplit-crypto<br/>shares, DLEq, commit-reveal]
    zcash[zecsplit-zcash<br/>lock address, detection, PCZT sweep]
    btc[zecsplit-btc<br/>script, 5 txs, adaptor sigs, sources]
    protocol[zecsplit-protocol<br/>messages, state machines, sessions]
    store[zecsplit-store<br/>write-ahead SQLite]
    engine[zecsplitd<br/>taker engine + local API]
    maker[zecsplit-maker<br/>quotes + Bob runner]
    crypto --> zcash
    crypto --> btc
    crypto --> store
    zcash --> protocol
    btc --> protocol
    protocol --> engine
    store --> engine
    protocol --> maker
    store --> maker
```

- **The browser holds no keys.** The local engine runs the swap and owns every secret. The web UI only shows engine state.
- **Write-ahead.** Every state change is committed to SQLite before the message or broadcast it allows.
- **Two Bitcoin sources.** The taker trusts a Bitcoin fact only when two independent sources agree.

## Judge kit

Needs Rust 1.93, Docker and [`just`](https://github.com/casey/just).

Run a swap yourself in the hosted UI, against a private regtest world on your machine:

```bash
just judge          # devnet + maker + your engine, mines blocks, prints the UI link and a BTC address
just judge-pay '<zcash: link under the QR>'   # pays the QR from the devnet wallet
```

Or the scripted run, no browser:

```bash
just devnet-up      # Zebra regtest (NU7) + bitcoind + a funded shielded wallet
just demo           # 3 swaps between real engine and maker processes, privacy checks, then a refund; prints every txid
```

About 4 minutes once built (the first build of the Zcash crates takes longer).

## Development

```bash
just setup          # enable the git hooks
just check          # fmt, clippy, tests, cargo deny, secret scan (what CI runs)
just devnet-test    # every scenario on regtest through the real engine and maker
```

## Docs

Full documentation: **[docs/](docs/README.md)** in this repo, also at [zecsplit.gitbook.io/zecsplit-docs](https://zecsplit.gitbook.io/zecsplit-docs/).

| Start here | |
|---|---|
| [Core concepts](docs/core-concepts.md) | The idea in ten minutes |
| [Protocol](docs/protocol.md) | The spec, rule by rule |
| [Threat model](docs/security/threat-model.md) | 25 threats, their mitigations and tests |
| [Privacy](docs/security/privacy.md) | What each observer learns |
| [Paper](docs/resources/paper.md) | The protocol in paper form |

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Third-party work and notices: [CREDITS.md](CREDITS.md).
