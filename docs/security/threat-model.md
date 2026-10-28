# Threat model

What we protect, who might attack, and how each threat is handled.

Scope: v1. ZEC → BTC, a local taker engine, a hosted maker, Zcash Ironwood, Bitcoin P2WSH. Section numbers (§) refer to the [Protocol](../protocol.md) pages. Test IDs: `S-xx` scenarios, `ADV-xx` adversarial tests, `PRIV-xx` privacy tests.

## What we protect

| Asset                                                 | Owner      | Worst case                                               |
| ----------------------------------------------------- | ---------- | -------------------------------------------------------- |
| Alice's ZEC in the lock                               | Alice      | Bob takes it without paying                              |
| Bob's BTC in the lock                                 | Bob        | Alice takes it without paying                            |
| Secret shares `s_a`, `s_b`; Bitcoin keys `k_a`, `k_b` | Each party | Theft                                                    |
| Unlinkability of Alice's shielded funds               | Alice      | An observer links her ZEC to her BTC address or identity |
| Availability                                          | Both       | Funds stuck longer than the stated deadlines             |

## Who might attack

| Who                               | What they can do                                                                  |
| --------------------------------- | --------------------------------------------------------------------------------- |
| **Malicious counterparty**        | Deviate from the protocol, go silent, choose shares adaptively, race transactions |
| **Lying data source**             | Show fake or missing transactions, hold back data, log queries                    |
| **Network observer**              | See IPs and timing. TLS hides message contents.                                   |
| **Chain analyst**                 | See everything public on both chains                                              |
| **Miners**                        | Reorder and delay transactions, take bribes                                       |
| **Supply chain**                  | Ship a malicious crate or package                                                 |
| **Malware on the user's machine** | **Out of scope.** If the endpoint is compromised, all bets are off.               |

## Threats

| ID  | Threat                                                                                       | Impact                                | Mitigation                                                                                            | Test            |
| --- | -------------------------------------------------------------------------------------------- | ------------------------------------- | ----------------------------------------------------------------------------------------------------- | --------------- |
| T1  | **Rogue key.** Bob picks `S_b = X − S_a` after seeing `S_a`, so he alone knows the lock key. | Bob steals the ZEC                    | Commit before reveal (§3.1-R2), proofs of knowledge (§3.2-R3)                                         | ADV-01          |
| T2  | **Mismatched twins.** `S^P` and `S^S` hide different secrets.                                | ZEC stuck or BTC lost                 | DLEq proof (§3.2-R3); Bob re-checks `[s_a]·SpendAuthG = S_a^P` (§4.5-R2)                              | ADV-02          |
| T3  | **Fake BTC lock** shown to Alice by a lying source                                           | Alice loses the ZEC                   | Two independent sources plus confirmation depth (§4.2-R1)                                             | ADV-03          |
| T4  | **Fake ZEC lock** shown to Bob by a lying light server                                       | Bob loses the BTC                     | Bob checks only his own Zebra node (§4.3-R4)                                                          | ADV-04          |
| T5  | **Wrong ZEC amount** (short, over, split)                                                    | Underpayment                          | Sum and confirmations checked before `esig_B` (§4.3-R2); §5.5                                         | ADV-05, S-06    |
| T6  | **Redeem vs cancel race**                                                                    | Alice loses both                      | Margin rule (§4.4-R1), CPFP (§4.4-R3), two sources                                                    | ADV-06          |
| T7  | **Refund vs punish race**                                                                    | Bob loses both                        | Refund within 1 block of cancel, CPFP (§5.3-R5)                                                       | ADV-07          |
| T8  | **Griefing the maker**: lock BTC, never pay                                                  | Bob's BTC stuck until `t1`, plus fees | Caps, concurrency limit, quote rate limits, short `t1` on test networks                               | ADV-08          |
| T9  | **Griefing the taker**: never send `esig_B`                                                  | Alice's ZEC stuck until `t1`          | Bounded by §5.3; the UI shows exact deadlines                                                         | S-04, S-05      |
| T10 | **Crash or offline** at a critical moment                                                    | Missed deadline                       | Write-ahead (§6.3-R1), resume (§6.3-R2), recovery file (§6.4)                                         | S-07            |
| T11 | **Fee spike** stalls pre-signed transactions                                                 | Deadline races                        | Generous fixed fees, CPFP-able outputs, long `t2`                                                     | ADV-10          |
| T12 | **Malleability** of `tx_lock`                                                                | Pre-signed txs invalid, BTC hostage   | SegWit-only inputs (§3.4-R2)                                                                          | ADV-11          |
| T13 | **Cross-swap replay**                                                                        | Confusion, theft                      | `swap_id` in every domain (§3.2-R4, §7-R1)                                                            | ADV-12          |
| T14 | **Secret leakage** via logs, UI, crash dumps, database                                       | Theft                                 | Zeroizing secret types with no `Debug`, encrypted session store and recovery file, log scanning in CI | ADV-13, PRIV-03 |
| T15 | **Maker server compromised**                                                                 | Hot wallet drained                    | Caps, minimal hot wallet, per-swap keys, alerts                                                       | Ops             |
| T16 | **IP linkage** by the maker                                                                  | Privacy loss                          | Optional Tor (§7-R2)                                                                                  | PRIV-04         |
| T17 | **Timing correlation** across chains                                                         | Weak link                             | Disclosed; optional delay before paying                                                               | Privacy         |
| T18 | **Amount correlation**: the maker knows both amounts                                         | Maker can link                        | Disclosed; the maker is a counterparty by design                                                      | Privacy         |
| T19 | **Light server learns interest** on the refund path                                          | Weak link                             | Compact-block scan with local trial decryption; no `GetTransaction` for lock notes                    | PRIV-05         |
| T20 | **NU7 branch ID**: a tx signed for the old branch is invalid after activation                | Sweep fails                           | Branch ID read from the node at signing time                                                          | S-08            |
| T21 | **Quantum**: shared-key notes are not quantum-recoverable (ZIP 2005)                         | Theoretical                           | Notes live minutes to hours; disclosed                                                                | —               |
| T22 | **Supply chain**                                                                             | Anything                              | Exact pins, `cargo deny`, reviewed vendored `dleq`                                                    | CI              |
| T23 | **Fake maker / man in the middle**                                                           | Bad quotes                            | Pinned maker identity key, signed quotes (§2), TLS                                                    | ADV-14          |
| T24 | **Reorgs**                                                                                   | A confirmed lock disappears           | Confirmation depths; re-check before every irreversible step                                          | ADV-09          |
| T25 | **UI deception**: shown amounts differ from signed ones                                      | User pays the wrong amount            | The UI renders only the signed quote and engine state; the engine builds the payment URI              | UI test         |
