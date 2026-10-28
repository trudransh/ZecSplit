# Testnet evidence

Raw data for every public-chain transaction in the README's [Live on testnet](../../README.md#live-on-testnet), saved on 28 Oct 2026 so you can check our claims without trusting our words.

- `zcash/<txid>.json`: `getrawtransaction <txid> 1` from the maker's Zebra 7.0.0-rc.0 on Zcash testnet (`confirmations` removed, since it changes every block). `hex` is the full transaction.
- `bitcoin/<txid>.json`: `GET https://mempool.space/signet/api/tx/<txid>`.

Each file can be re-fetched: any Zebra testnet node, or the same mempool.space URL.

## Shielded only

```bash
jq -r '[.txid[0:8], (.vin|length), (.vout|length), (.vShieldedSpend|length), (.vShieldedOutput|length), (.vjoinsplit|length), (.orchard.actions|length), (.ironwood.actions|length)] | @tsv' zcash/*.json
```

Columns: txid, transparent inputs, transparent outputs, Sapling spends, Sapling outputs, Sprout, Orchard actions, Ironwood actions. Expect `0 0 0 0 0 0` then the Ironwood count for all 8:

| Swap | Taker pays ZEC | Maker sweeps ZEC |
|---|---|---|
| 1 | `62cfea06` (4: the payer's wallet chose its own note count) | `22451c4e` (2) |
| 2 | `2139bd7a` (2) | `9125fcb0` (2) |
| 3 | `abdb747f` (2) | `85af11f5` (2) |
| 4 (web UI) | `e3585ecb` (2) | `2e264fe9` (2) |

The payment's action count depends on the payer's wallet; the sweep is built by ZecSplit and always has 2.

## A timelock firing on a public chain

```bash
jq -r '[.txid[0:8], .status.block_height, (.vin|map(.sequence)|tostring)] | @tsv' bitcoin/*.json
```

Swap `d9c9794f` stopped after the maker locked BTC and before any ZEC was paid:

| Transaction | Block | Input `nSequence` |
|---|---|---|
| lock `d1dcc58c` | 328056 | final |
| cancel `efed9957` | 328092 | **36**: valid only 36 blocks after the lock (`t1`), mined in exactly that block |
| refund `1503cf53` | 328093 | final |

Every other input is `4294967293` (no relative timelock).
