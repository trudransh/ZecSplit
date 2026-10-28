# Judge guide

Three ways to check ZecSplit, from no install to a full swap on your own machine.

| Path | Needs | Time | What it proves |
|---|---|---|---|
| [A. Check the public chains](#a-check-the-public-chains) | `curl`, `jq` | 2 min | The swaps and the timelock refund happened on Zcash testnet and Bitcoin signet |
| [B. Check the cryptography](#b-check-the-cryptography-against-the-chain) | Rust | 10 min (first build) | Each testnet swap's proofs, and the key share recovered from the chain |
| [C. Run a swap yourself](#c-run-a-swap-yourself-on-a-local-devnet) | Rust, Docker, `just` | 15 min first time, 2 min per swap | The whole protocol, in the hosted web app, against nodes on your machine |

A live swap on testnet is not self-serve: our maker is reachable only through our own SSH tunnel, and a swap needs testnet ZEC, signet BTC and about an hour of Bitcoin confirmations. Paths A and B check the testnet swaps we ran; path C runs the same code end to end, locally.

## A. Check the public chains

Every transaction of the four testnet swaps and the refund is linked in the README's [live run table](../README.md#the-live-run-on-the-explorers). Bitcoin links open mempool.space (signet), Zcash links open zecblock (testnet).

The timelock refund, checked from your terminal. Swap `d9c9794f` was abandoned after the maker locked BTC; its cancel could only be mined 36 blocks after the lock:

```bash
M=https://mempool.space/signet/api/tx; L=d1dcc58c246451446b20f6a305f4e2697cbd0ad605d9e8a3ddfceb59fab3e5a8
curl -s $M/efed99572f863eb1782088da7a8e1b2ebe17b0f3265fd970325a04f8dd28659a | jq -r --arg l $L \
  --argjson h "$(curl -s $M/$L/status | jq .block_height)" \
  '"spends the lock: \(.vin[0].txid == $l), nSequence: \(.vin[0].sequence), mined \(.status.block_height - $h) blocks after it"'
```

Expected: `spends the lock: true, nSequence: 36, mined 36 blocks after it`.

Shielded only: the raw Zcash transactions, as a Zebra testnet node returns them, are in [`evidence/testnet/zcash`](../evidence/testnet/zcash). From a clone:

```bash
cd evidence/testnet
jq -r '[.txid[0:8], (.vin|length), (.vout|length), (.vShieldedSpend|length), (.vShieldedOutput|length), (.orchard.actions|length), (.ironwood.actions|length)] | @tsv' zcash/*.json
```

Expected: every row `0 0 0 0 0` (no transparent, Sapling or Orchard parts) and then its Ironwood action count. More in [`evidence/testnet/README.md`](../evidence/testnet/README.md).

## B. Check the cryptography against the chain

Needs Rust: install with [rustup](https://rustup.rs); the repo pins 1.93.0 and rustup fetches it on first build. And [`just`](https://github.com/casey/just) (`cargo install just`), or run the `cargo` line below directly.

```bash
git clone https://github.com/trudransh/ZecSplit && cd ZecSplit
just verify
# or: cargo run --release -p zecsplitd --bin zecsplit-verify -- evidence/testnet/records/550dc7bd.json
```

The first build compiles the Zcash crates (several minutes). For each of the four testnet swaps it prints:

```
ok  Alice's and Bob's reveals open their commitments
ok  both cross-group DLEq proofs: each Zcash share point has the same secret as its Bitcoin twin
ok  all six proofs of knowledge
ok  tx_redeem pays Alice's committed payout address
ok  Bob's signature in tx_redeem, with his adaptor signature, yields s_a;
    s_a opens Alice's committed Zcash share point, so Bob could sweep the ZEC
```

`tx_redeem` is fetched from mempool.space, so this checks our records against the public chain. Edit one hex digit in a record and it fails.

## C. Run a swap yourself on a local devnet

Everything runs on your machine: Zebra (regtest, NU7), bitcoind (regtest), a shielded wallet, a maker and your engine. No coins needed. The web page at zecsplit.vercel.app is only a screen; it talks to the engine on `127.0.0.1`.

### Requirements

- Linux, macOS, or Windows with WSL 2.
- Docker with Compose, running.
- Rust ([rustup](https://rustup.rs)) and [`just`](https://github.com/casey/just).
- Chrome, Edge or Firefox. Free local ports 7780, 7781, 9067, 18232, 18443.

### Run

```bash
git clone https://github.com/trudransh/ZecSplit && cd ZecSplit
just judge
```

The first run builds the devnet images and the binaries (10 minutes or more); later runs start in about a minute. When ready it prints:

```
1. Open:            https://zecsplit.vercel.app/swap#engine=http://127.0.0.1:7780&token=…
2. Passphrase:      anything (it encrypts your recovery file)
3. BTC address:     bcrt1q…
4. Pay the QR:      just judge-pay '<the zcash: link under the QR>'  (in a second terminal)
```

1. Open the link. If the browser asks to allow access to devices on your local network, allow it: that is the page reaching your engine.
2. Enter any passphrase, ask for a quote (0.01 to 2 ZEC), paste the printed BTC address, read the risks and tick *I understand*.
3. Save the recovery file and confirm. The maker locks BTC; the QR appears once the lock confirms (about 30 s).
4. In a second terminal, from the repo: `just judge-pay '<the zcash: link under the QR>'`. Pay within about 7 minutes of the QR showing.
5. Watch the steps go green: the maker verifies the ZEC, your engine claims the BTC, the maker sweeps the ZEC. About a minute.

Blocks are mined every 10 s (Zcash) and 30 s (Bitcoin). Ctrl-C stops the maker, engine and miner; `just devnet-down` stops the nodes.

### The scripted run

No browser: three swaps between the real engine and maker, the privacy checks, then a refund, with every txid printed.

```bash
just devnet-up
just demo
```

### If something goes wrong

| Symptom | Fix |
|---|---|
| `Cannot connect to the Docker daemon` | Start Docker, then run again |
| A port is already in use | Stop what uses it, or `just devnet-down` from an earlier run |
| The page says it can't reach the engine | Open the exact link printed by `just judge` (the token changes every run); allow local network access if asked |
| `maker_failed` on *Start swap* | Click *Back*, get a new quote and start again |
| The QR disappeared before you paid | The pay window closed; the swap refunds by itself. Start a new one |
| The first build is slow | Expected: the Zcash crates are large. Later builds are incremental |
