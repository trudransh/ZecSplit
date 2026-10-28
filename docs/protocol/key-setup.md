# Key setup

Commit, reveal and verify, then derive the shared Zcash key and lock address (spec §3.1–§3.3).

## 3.1 Commit

**§3.1-R1.** Each party generates its share and keys, then sends a commitment:

```
C_x = H256(HK_commit_v1, swap_id ‖ role_byte ‖ S_x^P ‖ S_x^S ‖ K_x ‖ nonce_x)
```

where `nonce_x` is 32 random bytes.

**§3.1-R2.** Alice sends `C_A`; Bob replies with `C_B`. **Neither party reveals anything before holding the other's commitment.** This prevents rogue-key attacks: nobody can pick a share as a function of the other's.

## 3.2 Reveal and verify

**§3.2-R1.** Alice sends `Reveal_A`, then Bob sends `Reveal_B`. Each reveal contains:

* `S^P`, `S^S`, `K`, `nonce`;
* the DLEq proof `π_dleq`;
* the proofs of knowledge `pok_P`, `pok_S`, `pok_K`;
* the party's Bitcoin payout address (Alice: redeem and punish; Bob: refund).

**§3.2-R2.** Shares MUST be generated with the cross-group DLEq prover (Serai `dleq`, `CompromiseLinearDLEq`, vendored unmodified). The prover draws a scalar valid in both groups and returns the matching pair `(s^P, s^S)`.

* Transcript: domain `HK_dleq_v1`, then `swap_id` (32 bytes), then `role_byte` (Alice 0, Bob 1).
* Primary generators: `SpendAuthG` on Pallas, `G` on secp256k1.
* Alternate generators: hashed to curve from domain `HK_dleq_alt_v1` (Pallas `hash_to_curve`; secp256k1 RFC 9380 `secp256k1_XMD:SHA-256_SSWU_RO_`). Their encodings are pinned by a test.

**§3.2-R3.** Each party MUST verify the other's reveal, and abort the swap (no retry with the same `swap_id`) if any check fails:

1. the reveal matches the commitment;
2. `π_dleq` verifies for `(S^P, S^S)`;
3. `pok_P` is a RedPallas SpendAuth signature, valid under `S^P`, over `m_pok = H256(HK_pok_v1, swap_id ‖ role_byte ‖ C_A ‖ C_B)`;
4. `pok_S` is a BIP340 signature valid under `S^S` over `m_pok`;
5. `pok_K` is an ECDSA signature valid under `K` over `m_pok`;
6. no point is the identity;
7. `S_a ≠ S_b` in both groups.

**§3.2-R4.** Both sides compute the transcript identically:

```
T = H256(HK_transcript_v1, protocol_version ‖ swap_id ‖ network ‖ quote_hash ‖ C_A ‖ C_B ‖ Reveal_A ‖ Reveal_B)
```

Every later signature domain includes `swap_id`, so nothing can be replayed across swaps.

Byte layout: `protocol_version` u16 LE; `network` one byte (regtest 0, test 1, main\_beta 2); `quote_hash = H256(HK_quote_v1, canonical_json(quote))`; each `Reveal` is `S^P (32) ‖ S^S (33) ‖ K (33) ‖ nonce (32) ‖ u32 LE len ‖ π_dleq ‖ pok_P (64) ‖ pok_S (64) ‖ pok_K (64) ‖ u32 LE len ‖ payout_addr (UTF-8)`.

## 3.3 Shared Zcash key and lock address

**§3.3-R1 (the σ rule).** Let `P = S_a^P + S_b^P`.

* If the sign bit of `P`'s encoding is 0 (`ỹ = 0`), `σ = +1`; otherwise `σ = −1`.
* `ak = [σ]·P`, which satisfies Orchard's `ỹ(ak) = 0` rule.
* The spend-authorizing key is `ask = σ·(s_a + s_b) mod q`.
* Shares are **not** negated individually. `σ` is applied only when signing.

**§3.3-R2.** Neither party chooses the viewing keys. Both are derived from the transcript:

```
nk   = F_p.from_uniform_bytes(H512(HK_nk_v1, T))
rivk = F_q.from_uniform_bytes(H512(HK_rivk_v1, T))
```

**§3.3-R3.** `FVK = FullViewingKey::from_bytes(ak ‖ nk ‖ rivk)`. If that fails, or the derived `ivk` is invalid, abort. The probability is negligible, so a failure signals a bug.

**§3.3-R4.** The lock address is `fvk.address_at(0, External)`, encoded as a Unified Address with **only** an Orchard receiver, so funds land in the Ironwood pool. The UFVK is Orchard-only too.

**§3.3-R5.** The UFVK and `T` stay between Alice and Bob. They MUST NOT be sent to third parties, except to a node the party itself runs.
