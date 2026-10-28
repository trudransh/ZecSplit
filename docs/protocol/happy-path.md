# Happy path

Bob locks BTC, Alice pays ZEC, Alice redeems, Bob sweeps (spec §4).

## 4.1 Bob locks BTC

* **R1.** Bob signs and broadcasts `tx_lock` and waits for `n_conf_btc` confirmations.

## 4.2 Alice verifies the BTC lock

* **R1.** Alice MUST confirm `tx_lock` on **at least two independent Bitcoin data sources**. Both must agree on the txid, the output script `P2WSH(W)`, the value `V_lock`, and a confirmation depth of at least `n_conf_btc`.
* **R2.** Only then MAY the UI show the payment request.

## 4.2b Alice pays ZEC

* **R1.** The UI shows `zcash:<lock_UA>?amount=<zec_amount>`, a ZIP 321 URI with no memo. Alice pays it from any Zcash wallet.
* **R2.** The engine MUST NOT pay from a transparent source. In v1 the engine doesn't hold Alice's ZEC at all.

## 4.3 Bob verifies the ZEC lock

* **R1.** Bob scans with the lock UFVK against **his own Zebra node**.
* **R2.** He accepts only if the unspent Ironwood notes at the lock address add up to at least `zec_amount`, each with at least `n_conf_zec` confirmations.
* **R3.** Then he sends `esig_B(tx_redeem; Y_a)` to Alice.
* **R4.** Bob MUST NOT use a light server he doesn't control for this check.

## 4.4 Alice claims the BTC

* **R1 (margin rule).** Alice MUST NOT broadcast `tx_redeem` unless `current_height + redeem_margin < h_lock + t1`, where `h_lock` is `tx_lock`'s confirmation height. Inside the margin she cancels instead (see Failure paths).
* **R2.** She verifies `esig_B`, decrypts it with `s_a` to get `sig_B(tx_redeem)`, adds her own signature, and broadcasts to at least two sources.
* **R3.** If it isn't confirmed after 3 blocks, Alice MUST fee-bump it with CPFP from her redeem output.
* **R4.** Done when `tx_redeem` has at least `n_conf_btc` confirmations.

> [!NOTE]
> **Why the margin?** A redeem that reveals `s_a` but doesn't confirm before `t1` would let Bob cancel and refund while also knowing `s_a`. Bob would end up with both coins.

## 4.5 Bob takes the ZEC

* **R1.** Bob watches for `tx_redeem`, takes Alice's decrypted signature from its witness, and recovers `s_a = recover(Y_a, sig_published, esig_B)`.
* **R2.** He MUST check that `[s_a]·SpendAuthG = S_a^P`.
* **R3.** He computes `ask = σ·(s_a + s_b)`, builds a PCZT that spends every lock-address note to his own address, and signs each spend with `rsk_i = ask + α_i`. Then he proves, extracts and broadcasts.
* **R4.** Bob has **no deadline** on Zcash. Alice knows only `s_a`, so she can never move the note.
