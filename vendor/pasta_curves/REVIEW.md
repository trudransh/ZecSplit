# Vendored `pasta_curves` 0.5.1: review notes

T-015. Why this crate is vendored at all: `vendor/dleq/REVIEW.md` ("Plan A or Plan B").

## Provenance

- crates.io `pasta_curves` 0.5.1 (MIT OR Apache-2.0, Electric Coin Co.), from `zcash/pasta_curves`
  commit `7e3fc6a4919f6462a32b79dd226cb2587b7961eb` (see `.cargo_vcs_info.json`).
- `.crate` SHA-256 `d3e57598f73cc7e1b2ac63c79c517b31a0877cd7c402cdcaa311b5208de7a095`, the same as
  the `cksum` in the crates.io index.
- Contents: the extracted `.crate` minus cargo's `.cargo-ok` marker. The crate's own `.gitignore`
  ignores `*.html`, so `katex-header.html` (a docs.rs header) was added with `git add -f`.
  `rust-toolchain.toml` (1.56.0) only applies when cargo runs inside this directory. Building
  from the workspace root uses the root toolchain.

## Changes versus upstream

The first commit vendors the crate verbatim. The next one is the only change: a backport of the
`zeroize` feature that pasta_curves 0.6.0 added upstream (its CHANGELOG: "`zeroize` feature
flag, which enables `impl zeroize::DefaultIsZeroes` for `Fp`, `Fq`, `Ep`, `EpAffine`, `Eq` and
`EqAffine`"). The text is copied from 0.6.1.

| File | Change |
|---|---|
| `Cargo.toml` | Optional `zeroize = "1"` dependency (no default features) and the `zeroize = ["dep:zeroize"]` feature. |
| `src/fields/fp.rs`, `src/fields/fq.rs` | `#[cfg(feature = "zeroize")] impl zeroize::DefaultIsZeroes for Fp/Fq {}`, plus 0.6.1's `test_zeroize`. |
| `src/curves.rs` | The same impl for `$name` and `$name_affine` inside `new_curve_impl!` (so for `Ep`, `Eq`, `EpAffine`, `EqAffine`), plus 0.6.1's `zeroize_tests`. One import differs: group 0.13 provides `generator()` on affine points through `group::prime::PrimeCurveAffine`. |

`Cargo.toml.orig` is left as upstream; cargo doesn't read it.

**Why it is safe:** `DefaultIsZeroes` requires `Copy + Default`, and both impls already exist in
0.5.1: zero for the fields, the identity for the points. Zeroizing just overwrites a value with
`Default::default()`. No arithmetic, encoding or constant changed.

## Checks

In a scratch copy, `cargo test --lib --features zeroize,alloc` gives 33 passed: upstream's 29 plus
the 4 backported zeroize tests. With default features, upstream's 29 pass. Re-check the diff:

```sh
diff -ru -x REVIEW.md -x .cargo-ok ~/.cargo/registry/src/*/pasta_curves-0.5.1 vendor/pasta_curves
```
