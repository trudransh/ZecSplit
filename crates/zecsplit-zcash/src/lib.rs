//! Shared FVK/UFVK and lock UA, ZIP 321 URI, lock detection, sweep via PCZT.

mod lock;
mod watch;

pub use lock::{LockKeyError, LockKeys, lock_keys, regtest};
pub use watch::{LockStatus, WatchError, lock_status};

#[cfg(test)]
mod tests {
    use pczt::roles::{creator::Creator, signer::Signer};
    use zcash_protocol::consensus::BranchId;
    use zcash_protocol::constants::testnet::COIN_TYPE;

    /// T-002: the pinned pre-release set builds a v6 PCZT for NU7 and a Signer over it, the
    /// role that applies the externally made spend signatures in §4.5-R3.
    #[test]
    fn t_002_pinned_set_builds_nu7_signer() {
        let pczt = Creator::new(u32::from(BranchId::Nu7), 0, COIN_TYPE, None, None)
            .unwrap()
            .build()
            .unwrap();
        let signer = Signer::new(pczt).unwrap();
        assert_ne!(signer.shielded_sighash(), [0; 32]);
    }

    /// §3.3-R3 relies on `from_bytes` refusing invalid key material.
    #[test]
    fn fvk_from_bytes_rejects_all_zero_key() {
        assert!(orchard::keys::FullViewingKey::from_bytes(&[0; 96]).is_none());
    }
}
