# Vendored `dleq` 0.4.1: review notes

T-015, PROTOCOL_SPEC §3.2-R2, ADR-005, THREAT_MODEL T22.

## Provenance

- crates.io `dleq` 0.4.1 (MIT, Luke Parker / Serai), from `serai-dex/serai` commit
  `dd523b22c2a951601ba94729ff84d37b72d56fc7`, path `crypto/dleq` (see `.cargo_vcs_info.json`).
- `.crate` SHA-256 `b8d3af08cb0a3253930b6b9629fd61202ea34da117940b59ab58f3e6f5e1ba6c`, the same as
  the `cksum` in the crates.io index.
- Contents: the extracted `.crate`, minus cargo's `.cargo-ok` marker. `LICENSE` is upstream's.

## Changes versus upstream

**None.** Every file except this `REVIEW.md` is byte-identical to the published crate. To check:

```sh
cargo fetch   # in any project that depends on dleq =0.4.1
diff -r -x REVIEW.md -x .cargo-ok ~/.cargo/registry/src/*/dleq-0.4.1 vendor/dleq
```

`vendor/rustfmt.toml` (`disable_all_formatting`) stops `cargo fmt --all`, which also walks path
dependencies, from reformatting vendored code. The root `Cargo.toml` excludes the vendored
crates from the workspace, so our lints and `clippy -D warnings` don't apply to them.

## Plan A or Plan B: why the proof code stays on group 0.13

`dleq` needs both groups to be `PrimeGroup + Zeroize` with `Scalar: PrimeFieldBits + Zeroize`.
So does its dependency `multiexp` (Serai), whose `BatchVerifier` runs `to_le_bits` on the
scalars of both groups during verification.

- **Plan A (bump to group/ff 0.14) does not work as specified.** `k256` 0.14.0 is the only stable
  secp256k1 on group 0.14, and it no longer implements `PrimeFieldBits`: the `bits` feature was
  removed in 0.14.0-rc.10. `elliptic-curve` 0.14.1 and `primeorder` 0.14.0 don't provide it
  either. Every `elliptic-curve` 0.14 prerelease that still has `bits` (up to rc.32) is built on
  the `rustcrypto-ff` fork, not zkcrypto `ff` 0.14, so rc.9, the last k256 with `bits`, doesn't
  help. `multiexp` is still on group 0.13 at its newest release (0.5.2). Plan A would mean three
  vendored crates: `dleq` and `multiexp` ported to group 0.14 and rand_core 0.10, plus `k256`
  0.14 with a new `PrimeFieldBits` impl. That impl's bit decomposition feeds the multiexp
  verification math, so it would be new code on the critical path.
- **Plan B (stay on 0.13, bridge through bytes) works with one hygiene-only patch.** `k256`
  0.13.4 (`bits`) meets every bound unmodified. `pasta_curves` 0.5.x (group 0.13) meets every
  bound except `Zeroize`, which arrived only in pasta 0.6.0. `vendor/pasta_curves` is 0.5.1 with
  that feature backported verbatim (`impl DefaultIsZeroes` for the fields and points; see its
  `REVIEW.md`). It changes no arithmetic.

Plan B leaves the proof code, `multiexp` and `k256` exactly as upstream ships them. Pallas values
cross to pasta 0.6 / orchard only as canonical 32-byte encodings (same curve, same bytes).

## How it is used (`crates/zecsplit-crypto/src/dleq.rs`)

- Features `experimental` (the `cross_group` module) and `serialize` (proof bytes). The default
  `secure_capacity_difference` stays on. The capacities are 254 (Pallas `F_q`) and 255
  (secp256k1), so the shared capacity is 254 bits.
- `G0` = Pallas from pasta 0.5.1, `G1` = secp256k1 from `k256` 0.13.4.
- Variant `CompromiseLinearDLEq`, which upstream recommends. Proof size: 49,151 bytes. Verify
  takes about 0.2 s in a release build.
- Generators: primary `SpendAuthG` = `hash_to_curve("z.cash:Orchard")(b"G")` (a test pins it to
  reddsa's Orchard basepoint) and secp256k1's `G`. Alternates: Pallas
  `hash_to_curve("HK_dleq_alt_v1")(b"")`; secp256k1 RFC 9380
  `secp256k1_XMD:SHA-256_SSWU_RO_` with DST `HK_dleq_alt_v1` and an empty message.
- `Generators::new` accepts `SpendAuthG` as primary (it only rejects `primary == alt`), and
  proofs over it verify. This resolves the T-015 unknown in CODE_FINDINGS §6.
- Transcript: `RecommendedTranscript::new(b"HK_dleq_v1")`, then `append_message(b"swap_id", ..)`
  and `append_message(b"role_byte", [role])`.
- The scalar comes from `prove`'s digest path (SHA-512 over 32 fresh RNG bytes, truncated to 254
  bits).

## Upstream tests

The published manifest drops two dev-dependencies (`dalek-ff-group` and `transcript` with
`recommended`), so `cargo test` inside `vendor/dleq` does not compile as shipped. I ran the suite
in a scratch copy with those two restored from `Cargo.toml.orig` (`dalek-ff-group` 0.4.6):
`cargo test --features experimental,serialize` gave 11 passed and 4 ignored (benchmarks).

## For the reviewer

1. Confirm that nothing changed (`diff` above).
2. The choices in `dleq.rs`: the variant, the transcript layout, and the alternate-generator
   domains and messages. These are wire-level constants, and the spec doesn't fix them yet.
3. PROTOCOL_SPEC §1 says shares lie in `[1, 2^252)`. `dleq` produces `[0, 2^254)`, because 254
   bits is the Pallas/secp256k1 shared capacity; 2^252 is the ed25519 figure from the Monero
   original.
