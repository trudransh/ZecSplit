# Testing

How ZecSplit is tested, and the scenario scoreboard.

* Every test that covers a spec rule carries the rule ID in its name, e.g. `spec_4_4_r1_refuses_redeem_inside_margin`.
* `just check` runs `cargo fmt --check`, `clippy -D warnings`, every test, `cargo deny` and a secret scan. CI runs the same command.
* `just devnet-test` runs every scenario on a live Zebra regtest and bitcoind, through the real engine and maker.
* `just demo` is the judge kit: three swaps between real engine and maker processes with the privacy checks, then the refund path.

## Scenarios

| ID   | Scenario                                                              | Status                                                                                                                          |
| ---- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| S-01 | Happy path                                                            | Passes on regtest (engine and maker as separate processes over HTTP, 3 in a row) and **live on Zcash testnet + Bitcoin signet** |
| S-02 | Maker never locks BTC                                                 | Passes on regtest: the taker aborts with nothing paid                                                                           |
| S-03 | Taker never pays ZEC → maker cancels and refunds                      | Passes on regtest, and live on testnet                                                                                          |
| S-04 | Maker silent after the ZEC lock → cancel → refund → taker sweeps back | Passes on regtest, with a real ZEC sweep                                                                                        |
| S-05 | Maker never refunds → punish                                          | Passes on regtest                                                                                                               |
| S-06 | Wrong ZEC amount (overpay)                                            | Passes on regtest: no redeem signature, both refund, the taker sweeps it all back                                               |
| S-07 | Crash and resume at every state                                       | Passes in simulation and on regtest (both sides rebuilt from SQLite before every tick)                                          |
| S-08 | NU7 on the public network                                             | Passes: the live testnet swap's lock and sweep were mined after NU7                                                             |

## Adversarial tests

| ID              | Attack                                                          | Status                                                      |
| --------------- | --------------------------------------------------------------- | ----------------------------------------------------------- |
| ADV-01          | Rogue key                                                       | Rejected                                                    |
| ADV-02          | Mismatched twins                                                | Rejected                                                    |
| ADV-03          | Fake BTC lock (sources disagree)                                | No payment request                                          |
| ADV-04          | Maker checks the ZEC lock on a server it doesn't own            | The maker refuses to start                                  |
| ADV-05          | Underpay, overpay or wrong pool                                 | No redeem signature; both refund                            |
| ADV-06          | Redeem signature inside the taker's margin                      | Not redeemed; cancel path, the taker's share never revealed |
| ADV-07          | Maker's refund delayed or blocked                               | The taker punishes at `t2`                                  |
| ADV-08          | Quote flooding and abandoned setups                             | Capped at the per-network swap limit; stalled setups expire |
| ADV-09          | Reorg of the BTC lock                                           | Payment request hidden; the taker never aborts after paying |
| ADV-14          | Quote signed by another key                                     | Rejected                                                    |
| ADV-10 … ADV-13 | See the [threat model](../security/threat-model.md) | Planned                                                     |

## Model check

Both state machines are pure, so they are driven through 20,000 random runs each (60 steps, with reorgs, disagreeing Bitcoin sources, block jumps and hostile peer messages). After every step the test checks the rules that keep funds safe:

* no payment request without a confirmed lock outside the safety margin;
* no first redeem inside the margin;
* no cancel against the taker's own pending redeem;
* a sweep only once the other side's share is public;
* the maker locks only after the taker's signatures, and signs the redeem only on the exact, confirmed ZEC lock;
* finished states never move;
* nobody aborts after paying (taker) or locking (maker).

It found one real bug: a reorg of the BTC lock could let a taker who had already paid abort. Fixed before release.

## Fuzzing

* Mutated setup signatures, refund adaptor signatures, redeem adaptor signatures, lock templates and reveals are all rejected, before and after the genuine message, and a rejection never leaves the session unusable.
* The wire decoder and the recovery-file parser take 60,000 random and mutated inputs without a panic.

## Privacy tests

| ID      | Test                                                                    | Status                |
| ------- | ----------------------------------------------------------------------- | --------------------- |
| PRIV-01 | Every Zcash transaction has zero transparent inputs and outputs         | Passes on real chains |
| PRIV-02 | Lock and sweep look like ordinary 2-action Ironwood transactions        | Passes on real chains |
| PRIV-03 | No secret appears in any file the engine or maker writes, logs included | Passes                |
| PRIV-05 | Nothing asks a light server for a transaction by its id                 | Passes                |
