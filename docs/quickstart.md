# Quickstart

Run full ZEC ↔ BTC swaps on a local regtest devnet in one command.

The devnet runs a Zebra regtest node with NU7 active, a light-wallet endpoint, a Bitcoin Core regtest node, and a funded shielded Zcash wallet, all in Docker.

## Requirements

* Rust 1.93 (pinned by `rust-toolchain.toml`)
* Docker with Compose
* [`just`](https://github.com/casey/just)

## Swap in the web UI

```bash
git clone https://github.com/trudransh/ZecSplit
cd ZecSplit
just judge          # devnet + a maker + your engine, mines blocks, prints the UI link and a BTC address
```

Open the printed link: it is the hosted UI at [zecsplit.vercel.app](https://zecsplit.vercel.app), talking only to the engine on your machine. Pick an amount, paste the printed BTC address, save the recovery file, and when the QR appears pay it from the devnet wallet in a second terminal:

```bash
just judge-pay '<the zcash: link under the QR>'
```

Blocks are mined every 10 s (ZEC) and 30 s (BTC); pay within about 7 minutes of the QR showing. Ctrl-C stops everything.

## Scripted run

```bash
git clone https://github.com/trudransh/ZecSplit
cd ZecSplit
just devnet-up      # Zebra regtest + bitcoind + a funded shielded wallet
just demo           # 3 swaps between real engine and maker processes, privacy checks, then a refund
```

`just demo` takes about 4 minutes once built; the first build of the Zcash crates takes longer. It prints every transaction id, for example:

```
round 1 swap 73b834aa…: ["tx_redeem fbbd82ec…", "tx_lock 49a1eb97…", "zec_sweep 2b695dc6…"]
refund path: lock f4d48b1c…, cancel c18813e1…, refund 51e9de21…
```

Each round is a full swap between the engine (`zecsplitd`) and the maker (`zecsplit-maker`) as separate processes over HTTP: the maker locks BTC, a scripted wallet pays the ZEC QR, the engine redeems the BTC, and the maker sweeps the ZEC. After the three rounds the test checks every Zcash transaction for transparent parts and every file both programs wrote for secrets.

## Everything else

```bash
just setup          # enable the git hooks
just check          # fmt, clippy, tests, cargo deny, secret scan (what CI runs)
just devnet-test    # every scenario on regtest through the real engine and maker
just devnet-down    # stop the stack
```

## Testnet

The same binaries run on Zcash testnet and Bitcoin signet. `ops/testnet` holds the maker's node stack (Zebra on testnet with its light server, bitcoind on signet) and `ops/testnet/swap.sh`, which starts the maker on that machine and the engine locally over an SSH tunnel, then runs swaps through the engine API exactly as the UI does. See [Running a maker](developers/running-a-maker.md).
