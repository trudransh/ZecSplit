# Network constants

Timelocks, confirmations and limits per network (spec §8).

| Constant                   | `regtest` | `test` (Zcash testnet + Bitcoin signet) |
| -------------------------- | --------- | --------------------------------------- |
| `t1`                       | 20 blocks | 36 (\~6 h)                              |
| `t2`                       | 20 blocks | 36                                      |
| `n_conf_btc`               | 1         | 1                                       |
| `n_conf_zec`               | 1         | 5                                       |
| `redeem_margin`            | 5         | 12                                      |
| Fee floor (sat/vB)         | 2         | 2                                       |
| Max per swap               | none      | 2 ZEC                                   |
| Concurrent swaps per maker | 10        | 3                                       |

There is no mainnet configuration. A capped mainnet mode, if ever enabled, hard-codes its per-swap cap as a compile-time constant (§8-R1), not configuration.

Deadlines are block heights everywhere in the protocol. Wall-clock times appear only in the UI.
