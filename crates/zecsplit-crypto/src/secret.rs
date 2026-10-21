//! Secret newtypes (AGENTS.md NEVER-2, guard G-L1). Each is zeroed on drop and has no
//! `Debug`, `Display`, `Clone` or `Serialize`, so a secret can't be printed, logged,
//! copied or sent by accident. The only way to read one is `expose_secret`, which keeps
//! every use greppable.

use zeroize::{Zeroize, ZeroizeOnDrop};

macro_rules! secret_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Zeroize, ZeroizeOnDrop)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Takes ownership of `bytes`. The caller is responsible for zeroizing any copy
            /// it still holds.
            pub fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub fn expose_secret(&self) -> &[u8; 32] {
                &self.0
            }
        }
    };
}

secret_type!(
    /// A party's key share `s_x` (PROTOCOL_SPEC §1, ADR-028): 32 bytes little-endian, below 2^254,
    /// valid as a scalar on both Pallas and secp256k1.
    ///
    /// It can't be formatted, displayed or cloned:
    ///
    /// ```compile_fail,E0277
    /// let s = zecsplit_crypto::Share::from_bytes([1; 32]);
    /// let _ = format!("{s:?}");
    /// ```
    ///
    /// ```compile_fail,E0277
    /// let s = zecsplit_crypto::Share::from_bytes([1; 32]);
    /// let _ = format!("{s}");
    /// ```
    ///
    /// ```compile_fail,E0599
    /// let s = zecsplit_crypto::Share::from_bytes([1; 32]);
    /// let _ = s.clone();
    /// ```
    Share
);

secret_type!(
    /// The lock address's spend-authorizing key `ask = σ·(s_a + s_b) mod q` (§3.3-R1).
    /// Only the party that learns both shares ever builds one.
    Ask
);

secret_type!(
    /// A secp256k1 private key for the 2-of-2 lock (`k_a` or `k_b`, §1).
    BtcSecret
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_006_zeroize_clears_every_secret_type() {
        let mut share = Share::from_bytes([7; 32]);
        let mut ask = Ask::from_bytes([7; 32]);
        let mut btc = BtcSecret::from_bytes([7; 32]);
        share.zeroize();
        ask.zeroize();
        btc.zeroize();
        assert_eq!(share.expose_secret(), &[0; 32]);
        assert_eq!(ask.expose_secret(), &[0; 32]);
        assert_eq!(btc.expose_secret(), &[0; 32]);
    }
}
