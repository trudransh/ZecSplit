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
`zeroize` feature that **pasta_curves 0.6.0** added upstream (released 2026-09-25 from
`zcash/pasta_curves` commit `aa2fd598a28b35fe6741204b6df7362737866bf8`). Its CHANGELOG entry
reads: "`zeroize` feature flag, which enables `impl zeroize::DefaultIsZeroes` for `Fp`, `Fq`,
`Ep`, `EpAffine`, `Eq` and `EqAffine`. Zeroizing a field element sets it to zero; zeroizing a
point sets it to the identity." The impls match 0.6.0's text exactly. The tests are copied
from 0.6.1, with one import changed: group 0.13 provides `generator()` on affine points
through `group::prime::PrimeCurveAffine`, where 0.6.1 imports `group::CurveAffine`.

Only these four files differ from upstream; `Cargo.toml.orig` is left as upstream (cargo doesn't
read it). The exact diff:

```diff
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -102,6 +102,11 @@
 version = "2.3"
 default-features = false
 
+[dependencies.zeroize]
+version = "1"
+optional = true
+default-features = false
+
 [dev-dependencies.bincode]
 version = "1.3"
 
@@ -141,3 +146,4 @@
     "lazy_static",
 ]
 uninline-portable = []
+zeroize = ["dep:zeroize"]
--- a/src/fields/fp.rs
+++ b/src/fields/fp.rs
@@ -259,6 +259,10 @@
     }
 }
 
+#[cfg(feature = "zeroize")]
+#[cfg_attr(docsrs, doc(cfg(feature = "zeroize")))]
+impl zeroize::DefaultIsZeroes for Fp {}
+
 impl Fp {
     /// Returns zero, the additive identity.
     #[inline]
@@ -936,3 +940,13 @@
         ])
     );
 }
+
+#[cfg(feature = "zeroize")]
+#[test]
+fn test_zeroize() {
+    use zeroize::Zeroize;
+
+    let mut a = <Fp as ff::Field>::ONE;
+    a.zeroize();
+    assert_eq!(a, Fp::zero());
+}
--- a/src/fields/fq.rs
+++ b/src/fields/fq.rs
@@ -259,6 +259,10 @@
     }
 }
 
+#[cfg(feature = "zeroize")]
+#[cfg_attr(docsrs, doc(cfg(feature = "zeroize")))]
+impl zeroize::DefaultIsZeroes for Fq {}
+
 impl Fq {
     /// Returns zero, the additive identity.
     #[inline]
@@ -934,3 +938,13 @@
         ])
     );
 }
+
+#[cfg(feature = "zeroize")]
+#[test]
+fn test_zeroize() {
+    use zeroize::Zeroize;
+
+    let mut a = <Fq as ff::Field>::ONE;
+    a.zeroize();
+    assert_eq!(a, Fq::zero());
+}
--- a/src/curves.rs
+++ b/src/curves.rs
@@ -287,6 +287,11 @@
             }
         }
 
+        /// Zeroizing a point overwrites it with the identity, which is the `Default` value.
+        #[cfg(feature = "zeroize")]
+        #[cfg_attr(docsrs, doc(cfg(feature = "zeroize")))]
+        impl zeroize::DefaultIsZeroes for $name {}
+
         impl ConstantTimeEq for $name {
             fn ct_eq(&self, other: &Self) -> Choice {
                 // Is (xz^2, yz^3, z) equal to (x'z'^2, yz'^3, z') when converted to affine?
@@ -743,6 +748,11 @@
             }
         }
 
+        /// Zeroizing a point overwrites it with the identity, which is the `Default` value.
+        #[cfg(feature = "zeroize")]
+        #[cfg_attr(docsrs, doc(cfg(feature = "zeroize")))]
+        impl zeroize::DefaultIsZeroes for $name_affine {}
+
         impl<'a> From<&'a $name> for $name_affine {
             fn from(p: &'a $name) -> $name_affine {
                 p.to_affine()
@@ -1203,3 +1213,33 @@
         0x2b3483a1ee9a382f,
     ]);
 }
+
+#[cfg(all(test, feature = "zeroize"))]
+mod zeroize_tests {
+    use group::{prime::PrimeCurveAffine, Group};
+    use zeroize::Zeroize;
+
+    use super::{Ep, EpAffine, Eq, EqAffine};
+
+    #[test]
+    fn projective_points_zeroize_to_identity() {
+        let mut p = Ep::generator();
+        p.zeroize();
+        assert!(bool::from(p.is_identity()));
+
+        let mut q = Eq::generator();
+        q.zeroize();
+        assert!(bool::from(q.is_identity()));
+    }
+
+    #[test]
+    fn affine_points_zeroize_to_identity() {
+        let mut p = EpAffine::generator();
+        p.zeroize();
+        assert!(bool::from(p.is_identity()));
+
+        let mut q = EqAffine::generator();
+        q.zeroize();
+        assert!(bool::from(q.is_identity()));
+    }
+}
```

**Why it is safe:** `DefaultIsZeroes` requires `Copy + Default`, and both impls already exist in
0.5.1: zero for the fields, the identity for the points. Zeroizing just overwrites a value with
`Default::default()`. No arithmetic, encoding or constant changed.

## Checks

In a scratch copy, `cargo test --lib --features zeroize,alloc` gives 33 passed: upstream's 29 plus
the 4 backported zeroize tests. With default features, upstream's 29 pass. Re-check the diff:

```sh
diff -ru -x REVIEW.md -x .cargo-ok ~/.cargo/registry/src/*/pasta_curves-0.5.1 vendor/pasta_curves
```
