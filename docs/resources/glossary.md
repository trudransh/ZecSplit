# Glossary

Terms used across the ZecSplit docs.

| Term                     | Meaning                                                                                                     |
| ------------------------ | ----------------------------------------------------------------------------------------------------------- |
| **Adaptor signature**    | A signature encrypted under a point `Y`. Decrypting it needs `y`; seeing both versions reveals `y`.         |
| **`ak`, `ask`**          | Orchard's spend-validating public key and spend-authorizing secret key. In ZecSplit, `ask = σ·(s_a + s_b)`. |
| **Alice**                | The party with ZEC who wants BTC. The taker in v1.                                                          |
| **Bob**                  | The party with BTC who wants ZEC. The maker in v1.                                                          |
| **CPFP**                 | Child-pays-for-parent: speeding up a stuck transaction by spending its output with a higher fee.            |
| **DLEq**                 | A proof that two public keys, here on different curves, hide the same secret.                               |
| **Engine** (`zecsplitd`) | The taker's local program that runs swaps and holds keys.                                                   |
| **Ironwood**             | The Zcash shielded pool that Orchard-style notes land in after NU7.                                         |
| **Lock address**         | The shielded Unified Address, with only an Orchard receiver, derived from both shares and the transcript.   |
| **Maker**                | A service that quotes and completes swaps (Bob).                                                            |
| **PCZT**                 | Partially Created Zcash Transaction: lets the spend signature be added outside the wallet.                  |
| **Recovery file**        | Alice's encrypted backup that lets a fresh install finish any swap path.                                    |
| **Share** (`s_a`, `s_b`) | Each party's half of the Zcash spending key, valid on both curves.                                          |
| **σ**                    | `+1` or `−1`, chosen so the combined key satisfies Orchard's sign rule.                                     |
| **`t1`, `t2`**           | Relative timelocks in Bitcoin blocks: lock → cancel valid, and cancel → punish valid.                       |
| **Transcript `T`**       | A hash of everything exchanged in setup. Viewing keys are derived from it.                                  |
| **UFVK**                 | Unified Full Viewing Key: lets its holder see, but not spend, the lock address's notes.                     |
| **Write-ahead**          | Saving a state change before performing the action it allows.                                               |
| **ZIP 321**              | The Zcash payment request URI format (`zcash:<address>?amount=...`).                                        |
