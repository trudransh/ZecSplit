//! The swap transcript `T` (PROTOCOL_SPEC §3.2-R4). Both parties compute it from the same
//! public setup data; it seeds `nk` and `rivk` (§3.3-R2), so any disagreement about the
//! quote, the commitments or the reveals changes the lock address.
//!
//! `T = H256(HK_transcript_v1, m)`. Every integer is little-endian, and every
//! variable-length field carries a u32 length prefix, so `m` has exactly one parse:
//!
//! ```text
//! m        = protocol_version (u16) ‖ swap_id (32) ‖ network (1) ‖ quote_hash (32)
//!            ‖ C_A (32) ‖ C_B (32) ‖ reveal(Reveal_A) ‖ reveal(Reveal_B)
//! reveal() = S^P (32) ‖ S^S (33) ‖ K (33) ‖ nonce (32)
//!            ‖ len (u32) ‖ dleq ‖ pok_P (64) ‖ pok_S (64) ‖ pok_K (64)
//!            ‖ len (u32) ‖ payout_addr (UTF-8)
//! ```

use crate::hash::{HK_TRANSCRIPT_V1, h256};
use crate::{Commitment, Reveal};

/// The quote's `network` (§2) as it enters `T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Regtest,
    Test,
    MainBeta,
}

impl Network {
    pub fn byte(self) -> u8 {
        match self {
            Network::Regtest => 0,
            Network::Test => 1,
            Network::MainBeta => 2,
        }
    }
}

/// Everything `T` covers, in the order it is hashed.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptInput<'a> {
    pub protocol_version: u16,
    pub swap_id: [u8; 32],
    pub network: Network,
    pub quote_hash: [u8; 32],
    pub c_a: Commitment,
    pub c_b: Commitment,
    pub reveal_a: &'a Reveal,
    pub reveal_b: &'a Reveal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TranscriptError {
    #[error("a reveal field is longer than a u32 length prefix can describe")]
    FieldTooLong,
}

/// §3.2-R4.
pub fn transcript(input: &TranscriptInput) -> Result<[u8; 32], TranscriptError> {
    let mut m = Vec::with_capacity(
        2 + 32 + 1 + 32 + 64 + reveal_len(input.reveal_a) + reveal_len(input.reveal_b),
    );
    m.extend_from_slice(&input.protocol_version.to_le_bytes());
    m.extend_from_slice(&input.swap_id);
    m.push(input.network.byte());
    m.extend_from_slice(&input.quote_hash);
    m.extend_from_slice(&input.c_a.0);
    m.extend_from_slice(&input.c_b.0);
    push_reveal(&mut m, input.reveal_a)?;
    push_reveal(&mut m, input.reveal_b)?;
    Ok(h256(HK_TRANSCRIPT_V1, &m))
}

fn reveal_len(r: &Reveal) -> usize {
    32 + 33 + 33 + 32 + 4 + r.dleq.len() + 3 * 64 + 4 + r.payout_addr.len()
}

fn push_reveal(m: &mut Vec<u8>, r: &Reveal) -> Result<(), TranscriptError> {
    m.extend_from_slice(&r.share_pub.pallas);
    m.extend_from_slice(&r.share_pub.secp);
    m.extend_from_slice(&r.btc_key);
    m.extend_from_slice(&r.nonce);
    push_prefixed(m, &r.dleq)?;
    m.extend_from_slice(&r.pok_p);
    m.extend_from_slice(&r.pok_s);
    m.extend_from_slice(&r.pok_k);
    push_prefixed(m, r.payout_addr.as_bytes())
}

fn push_prefixed(m: &mut Vec<u8>, field: &[u8]) -> Result<(), TranscriptError> {
    let len = u32::try_from(field.len()).map_err(|_| TranscriptError::FieldTooLong)?;
    m.extend_from_slice(&len.to_le_bytes());
    m.extend_from_slice(field);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::SharePub;

    fn reveal(tag: u8, dleq_len: usize, payout_addr: &str) -> Reveal {
        Reveal {
            share_pub: SharePub {
                pallas: [tag; 32],
                secp: [tag + 1; 33],
            },
            btc_key: [tag + 2; 33],
            nonce: [tag + 3; 32],
            dleq: vec![tag + 4; dleq_len],
            pok_p: [tag + 5; 64],
            pok_s: [tag + 6; 64],
            pok_k: [tag + 7; 64],
            payout_addr: payout_addr.into(),
        }
    }

    fn t(ra: &Reveal, rb: &Reveal, edit: impl FnOnce(&mut TranscriptInput)) -> [u8; 32] {
        let mut input = TranscriptInput {
            protocol_version: 1,
            swap_id: [0x5a; 32],
            network: Network::Regtest,
            quote_hash: [0x9b; 32],
            c_a: Commitment([0xca; 32]),
            c_b: Commitment([0xcb; 32]),
            reveal_a: ra,
            reveal_b: rb,
        };
        edit(&mut input);
        transcript(&input).unwrap()
    }

    /// `m` without its length prefixes: the encoding a careless implementation would hash.
    fn unprefixed(ra: &Reveal, rb: &Reveal) -> Vec<u8> {
        let mut m = Vec::new();
        for r in [ra, rb] {
            for field in [
                &r.share_pub.pallas[..],
                &r.share_pub.secp,
                &r.btc_key,
                &r.nonce,
                &r.dleq,
                &r.pok_p,
                &r.pok_s,
                &r.pok_k,
                r.payout_addr.as_bytes(),
            ] {
                m.extend_from_slice(field);
            }
        }
        m
    }

    /// Cuts `bytes` into consecutive pieces of the given lengths; the last takes the rest.
    fn cut<'a>(bytes: &'a [u8], lens: &[usize]) -> Vec<&'a [u8]> {
        let mut rest = bytes;
        let mut out = Vec::new();
        for &len in lens {
            let (head, tail) = rest.split_at(len);
            out.push(head);
            rest = tail;
        }
        out.push(rest);
        out
    }

    #[test]
    fn spec_3_2_r4_transcript_is_deterministic() {
        let (a, b) = (reveal(0x10, 40, "alice-addr"), reveal(0x20, 40, "bob-addr"));
        assert_eq!(t(&a, &b, |_| {}), t(&a, &b, |_| {}));
    }

    /// Each single change, including swapping A and B, gives its own `T`.
    #[test]
    fn spec_3_2_r4_every_input_changes_t() {
        let (a, b) = (reveal(0x10, 40, "alice-addr"), reveal(0x20, 40, "bob-addr"));
        let mut seen = HashSet::new();
        let mut add = |label: &str, value: [u8; 32]| {
            assert!(seen.insert(value), "{label} collided with an earlier T");
        };

        add("base", t(&a, &b, |_| {}));
        add("version", t(&a, &b, |i| i.protocol_version = 2));
        add("swap_id", t(&a, &b, |i| i.swap_id[31] ^= 1));
        add("network test", t(&a, &b, |i| i.network = Network::Test));
        add(
            "network main_beta",
            t(&a, &b, |i| i.network = Network::MainBeta),
        );
        add("quote_hash", t(&a, &b, |i| i.quote_hash[0] ^= 1));
        add("c_a", t(&a, &b, |i| i.c_a.0[0] ^= 1));
        add("c_b", t(&a, &b, |i| i.c_b.0[0] ^= 1));
        add(
            "swapped commitments",
            t(&a, &b, |i| std::mem::swap(&mut i.c_a, &mut i.c_b)),
        );
        add("swapped reveals", t(&b, &a, |_| {}));

        type Edit = fn(&mut Reveal);
        let edits: [(&str, Edit); 11] = [
            ("S^P", |r| r.share_pub.pallas[0] ^= 1),
            ("S^S", |r| r.share_pub.secp[0] ^= 1),
            ("K", |r| r.btc_key[0] ^= 1),
            ("nonce", |r| r.nonce[0] ^= 1),
            ("dleq byte", |r| r.dleq[0] ^= 1),
            ("dleq longer", |r| r.dleq.push(0)),
            ("pok_P", |r| r.pok_p[0] ^= 1),
            ("pok_S", |r| r.pok_s[0] ^= 1),
            ("pok_K", |r| r.pok_k[0] ^= 1),
            ("payout_addr byte", |r| {
                r.payout_addr.replace_range(0..1, "X")
            }),
            ("payout_addr longer", |r| r.payout_addr.push('x')),
        ];
        for (field, edit) in edits {
            let mut a2 = a.clone();
            edit(&mut a2);
            add(&format!("reveal_a.{field}"), t(&a2, &b, |_| {}));
            let mut b2 = b.clone();
            edit(&mut b2);
            add(&format!("reveal_b.{field}"), t(&a, &b2, |_| {}));
        }
    }

    /// Bytes moved between payout_addr and the variable-length field next to it keep the
    /// unprefixed concatenation identical, so only the length prefixes tell the inputs apart.
    #[test]
    fn spec_3_2_r4_length_prefixes_prevent_field_shifting() {
        let (a, b) = (reveal(0x10, 40, "alice-addr"), reveal(0x20, 40, "bob-addr"));
        let k = 3;

        let tail: Vec<u8> = [
            &a.dleq[..],
            &a.pok_p,
            &a.pok_s,
            &a.pok_k,
            a.payout_addr.as_bytes(),
        ]
        .concat();
        let p = cut(&tail, &[a.dleq.len() + k, 64, 64, 64]);
        let mut a2 = a.clone();
        a2.dleq = p[0].to_vec();
        a2.pok_p.copy_from_slice(p[1]);
        a2.pok_s.copy_from_slice(p[2]);
        a2.pok_k.copy_from_slice(p[3]);
        a2.payout_addr = String::from_utf8(p[4].to_vec()).unwrap();
        assert_eq!(unprefixed(&a, &b), unprefixed(&a2, &b));
        assert_ne!(t(&a, &b, |_| {}), t(&a2, &b, |_| {}));

        let span: Vec<u8> = [
            a.payout_addr.as_bytes(),
            &b.share_pub.pallas,
            &b.share_pub.secp,
            &b.btc_key,
            &b.nonce,
            &b.dleq,
        ]
        .concat();
        let p = cut(&span, &[a.payout_addr.len() - k, 32, 33, 33, 32]);
        let (mut a3, mut b3) = (a.clone(), b.clone());
        a3.payout_addr = String::from_utf8(p[0].to_vec()).unwrap();
        b3.share_pub.pallas.copy_from_slice(p[1]);
        b3.share_pub.secp.copy_from_slice(p[2]);
        b3.btc_key.copy_from_slice(p[3]);
        b3.nonce.copy_from_slice(p[4]);
        b3.dleq = p[5].to_vec();
        assert_eq!(unprefixed(&a, &b), unprefixed(&a3, &b3));
        assert_ne!(t(&a, &b, |_| {}), t(&a3, &b3, |_| {}));
    }
}
