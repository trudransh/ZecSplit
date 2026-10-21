//! Domain-separated hashes (PROTOCOL_SPEC §1): BLAKE2b with a 16-byte personalization,
//! the ASCII tag padded with NULs. Changing a tag is a protocol change (AGENTS.md NEVER-4).

pub(crate) const HK_COMMIT_V1: &[u8; 16] = b"HK_commit_v1\0\0\0\0";
pub(crate) const HK_NK_V1: &[u8; 16] = b"HK_nk_v1\0\0\0\0\0\0\0\0";
pub(crate) const HK_POK_V1: &[u8; 16] = b"HK_pok_v1\0\0\0\0\0\0\0";
pub(crate) const HK_QUOTE_V1: &[u8; 16] = b"HK_quote_v1\0\0\0\0\0";
pub(crate) const HK_RIVK_V1: &[u8; 16] = b"HK_rivk_v1\0\0\0\0\0\0";
// Exactly 16 bytes, so no padding.
pub(crate) const HK_TRANSCRIPT_V1: &[u8; 16] = b"HK_transcript_v1";

/// `H256(tag, m)`.
pub(crate) fn h256(tag: &[u8; 16], msg: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    out.copy_from_slice(
        blake2b_simd::Params::new()
            .hash_length(32)
            .personal(tag)
            .hash(msg)
            .as_bytes(),
    );
    out
}

/// `H512(tag, m)`.
pub(crate) fn h512(tag: &[u8; 16], msg: &[u8]) -> [u8; 64] {
    *blake2b_simd::Params::new()
        .hash_length(64)
        .personal(tag)
        .hash(msg)
        .as_array()
}
