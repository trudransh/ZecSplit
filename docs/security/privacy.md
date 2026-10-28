# Privacy

What each observer learns during a swap, and the known leaks.

## What each observer learns (happy path)

| Observer                                  | Learns                                                                                                  | Does not learn                                                                                                  |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| **Zcash chain**                           | Two Ironwood transactions: Alice's payment into the lock and Bob's sweep. Their fees and action counts. | Amounts, addresses, any link between the two, any link to Bitcoin. **No transparent input or output anywhere.** |
| **Bitcoin chain**                         | A 2-of-2 P2WSH output and one spend of it. Amounts, timing, Alice's payout address.                     | That it is a Zcash swap (barring timing correlation), the ZEC amount, Alice's Zcash wallet                      |
| **Maker (Bob)**                           | Both amounts, timing, Alice's BTC payout address, Alice's IP unless she uses Tor                        | Alice's Zcash wallet, her balance, her other notes. He sees only notes sent to the lock address.                |
| **Taker (Alice)**                         | Bob's lock and refund addresses                                                                         | Bob's other funds                                                                                               |
| **Bitcoin data sources** (two)            | Alice's IP and the txids she queries                                                                    | —                                                                                                               |
| **Zcash light server** (refund path only) | The block ranges scanned                                                                                | Which note is Alice's (local trial decryption)                                                                  |
| **Network observer**                      | That Alice talks to the maker                                                                           | Message contents (TLS)                                                                                          |

## Compared with other ways out of shielded ZEC

| Path                  | Transparent ZEC touched?                 | Who can freeze?            |
| --------------------- | ---------------------------------------- | -------------------------- |
| **ZecSplit**          | **No**                                   | **Nobody**                 |
| Intent networks       | Yes: a fresh transparent deposit address | Solvers, compliance holds  |
| Vault-based protocols | Yes: the vault address                   | Validators or the protocol |
| Centralized exchanges | Usually: the deposit address             | The exchange               |

> [!NOTE]
> Footprint figures are measured on testnet before release and will replace these expected values.

## Known metadata leaks

| Leak                         | Severity    | v1 mitigation                                                         | Later                         |
| ---------------------------- | ----------- | --------------------------------------------------------------------- | ----------------------------- |
| The maker sees Alice's IP    | Medium      | Optional Tor SOCKS5                                                   | Tor by default                |
| Timing across chains         | Low–medium  | The user may wait before paying; many Ironwood transactions per block | Randomized delays             |
| The maker knows both amounts | Inherent    | Disclosed                                                             | Several makers                |
| Bitcoin address reuse        | User choice | Warn on reuse                                                         | —                             |
| Zcash refund scanning        | Low         | Compact-block scan, no `GetTransaction`                               | Private information retrieval |

## Privacy tests

| ID      | Test                                                                                                                      |
| ------- | ------------------------------------------------------------------------------------------------------------------------- |
| PRIV-01 | Every Zcash transaction in any scenario has **zero transparent inputs and outputs**                                       |
| PRIV-02 | Lock and sweep transactions look like ordinary Ironwood transactions (action count, no memo pattern)                      |
| PRIV-03 | Logs and databases contain no hex of `s_a`, `s_b`, `ask` or Bitcoin keys after the scenario suite                         |
| PRIV-04 | The engine talks only to allowlisted hosts: the maker, two Bitcoin sources, and the Zcash light server on the refund path |
| PRIV-05 | The engine never calls `GetTransaction` with a lock-note txid                                                             |
| PRIV-06 | The UI loads no third-party scripts or analytics                                                                          |
