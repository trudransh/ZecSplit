//! Passphrase-encrypted recovery file (PROTOCOL_SPEC §6.4).

use age::scrypt;
pub use age::secrecy::SecretString;
use zecsplit_crypto::{BtcSecret, Role, Share};
use zeroize::Zeroizing;

pub const TX_LOCK: &str = "tx_lock";
pub const TX_REDEEM: &str = "tx_redeem";
pub const TX_CANCEL: &str = "tx_cancel";
pub const TX_REFUND: &str = "tx_refund";
pub const TX_PUNISH: &str = "tx_punish";
pub const SIG_B_CANCEL: &str = "sig_b_cancel";
pub const SIG_B_PUNISH: &str = "sig_b_punish";
pub const ESIG_A_REFUND: &str = "esig_a_refund";

/// §6.4-R1: what Alice's file must hold before it may be written.
pub const ALICE_REQUIRED: [&str; 8] = [
    TX_LOCK,
    TX_REDEEM,
    TX_CANCEL,
    TX_REFUND,
    TX_PUNISH,
    SIG_B_CANCEL,
    SIG_B_PUNISH,
    ESIG_A_REFUND,
];

const MAGIC: &[u8; 6] = b"ZSREC\x01";
const MAX_FIELD: usize = 1 << 20;
const MAX_BLOBS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("recovery file is missing {0}")]
    Missing(&'static str),
    #[error("wrong passphrase or damaged file")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
    #[error("not a ZecSplit recovery file, or an unsupported version")]
    Format,
}

/// §6.4-R1: everything a fresh install needs to finish the swap.
pub struct RecoveryBundle {
    pub swap_id: [u8; 32],
    pub role: Role,
    pub quote_json: String,
    pub maker_endpoint: String,
    pub share: Share,
    pub btc_key: BtcSecret,
    pub transcript: [u8; 32],
    pub blobs: Vec<(String, Vec<u8>)>,
}

impl RecoveryBundle {
    /// The stored bytes under `name`, if any.
    pub fn blob(&self, name: &str) -> Option<&[u8]> {
        self.blobs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.as_slice())
    }
}

/// §6.4-R1: encrypts the bundle under `passphrase` with age/scrypt.
pub fn seal(bundle: &RecoveryBundle, passphrase: SecretString) -> Result<Vec<u8>, RecoveryError> {
    seal_with(bundle, scrypt::Recipient::new(passphrase))
}

fn seal_with(
    bundle: &RecoveryBundle,
    recipient: scrypt::Recipient,
) -> Result<Vec<u8>, RecoveryError> {
    if bundle.role == Role::Alice
        && let Some(missing) = ALICE_REQUIRED.iter().find(|n| bundle.blob(n).is_none())
    {
        return Err(RecoveryError::Missing(missing));
    }
    let plain = encode(bundle)?;
    age::encrypt(&recipient, &plain).map_err(|_| RecoveryError::Encrypt)
}

/// §6.4-R1: decrypts and parses a recovery file.
pub fn open(file: &[u8], passphrase: SecretString) -> Result<RecoveryBundle, RecoveryError> {
    let identity = scrypt::Identity::new(passphrase);
    let plain = Zeroizing::new(age::decrypt(&identity, file).map_err(|_| RecoveryError::Decrypt)?);
    decode(&plain)
}

fn put(out: &mut Vec<u8>, field: &[u8]) -> Result<(), RecoveryError> {
    if field.len() > MAX_FIELD {
        return Err(RecoveryError::Format);
    }
    out.extend_from_slice(&(field.len() as u32).to_le_bytes());
    out.extend_from_slice(field);
    Ok(())
}

fn encode(b: &RecoveryBundle) -> Result<Zeroizing<Vec<u8>>, RecoveryError> {
    if b.blobs.len() > MAX_BLOBS {
        return Err(RecoveryError::Format);
    }
    let mut out = Zeroizing::new(Vec::with_capacity(4096));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&b.swap_id);
    out.push(match b.role {
        Role::Alice => 0,
        Role::Bob => 1,
    });
    out.extend_from_slice(b.share.expose_secret());
    out.extend_from_slice(b.btc_key.expose_secret());
    out.extend_from_slice(&b.transcript);
    put(&mut out, b.quote_json.as_bytes())?;
    put(&mut out, b.maker_endpoint.as_bytes())?;
    out.push(b.blobs.len() as u8);
    for (name, bytes) in &b.blobs {
        put(&mut out, name.as_bytes())?;
        put(&mut out, bytes)?;
    }
    Ok(out)
}

/// A bounds-checked reader over the decrypted plaintext.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], RecoveryError> {
        if self.0.len() < n {
            return Err(RecoveryError::Format);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn array(&mut self) -> Result<[u8; 32], RecoveryError> {
        self.take(32)?.try_into().map_err(|_| RecoveryError::Format)
    }

    fn field(&mut self) -> Result<&'a [u8], RecoveryError> {
        let len_bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| RecoveryError::Format)?;
        let len = u32::from_le_bytes(len_bytes) as usize;
        if len > MAX_FIELD {
            return Err(RecoveryError::Format);
        }
        self.take(len)
    }

    fn text(&mut self) -> Result<String, RecoveryError> {
        String::from_utf8(self.field()?.to_vec()).map_err(|_| RecoveryError::Format)
    }
}

fn decode(plain: &[u8]) -> Result<RecoveryBundle, RecoveryError> {
    let mut r = Reader(plain);
    if r.take(MAGIC.len())? != MAGIC {
        return Err(RecoveryError::Format);
    }
    let swap_id = r.array()?;
    let role = match r.take(1)? {
        [0] => Role::Alice,
        [1] => Role::Bob,
        _ => return Err(RecoveryError::Format),
    };
    let share = Share::from_bytes(*Zeroizing::new(r.array()?));
    let btc_key = BtcSecret::from_bytes(*Zeroizing::new(r.array()?));
    let transcript = r.array()?;
    let quote_json = r.text()?;
    let maker_endpoint = r.text()?;
    let count = usize::from(r.take(1)?[0]);
    if count > MAX_BLOBS {
        return Err(RecoveryError::Format);
    }
    let mut blobs = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.text()?;
        blobs.push((name, r.field()?.to_vec()));
    }
    if !r.0.is_empty() {
        return Err(RecoveryError::Format);
    }
    Ok(RecoveryBundle {
        swap_id,
        role,
        quote_json,
        maker_endpoint,
        share,
        btc_key,
        transcript,
        blobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn fast(passphrase: &str) -> scrypt::Recipient {
        let mut r = scrypt::Recipient::new(pass(passphrase));
        r.set_work_factor(10);
        r
    }

    fn alice_bundle() -> RecoveryBundle {
        RecoveryBundle {
            swap_id: [7; 32],
            role: Role::Alice,
            quote_json: r#"{"btc_amount":150000}"#.into(),
            maker_endpoint: "http://127.0.0.1:7071".into(),
            share: Share::from_bytes([1; 32]),
            btc_key: BtcSecret::from_bytes([2; 32]),
            transcript: [3; 32],
            blobs: ALICE_REQUIRED
                .iter()
                .enumerate()
                .map(|(i, n)| ((*n).to_string(), vec![u8::try_from(i).unwrap(); 70]))
                .collect(),
        }
    }

    #[test]
    fn spec_6_4_r1_round_trip_restores_every_field() {
        let file = seal_with(&alice_bundle(), fast("correct horse")).unwrap();
        let back = open(&file, pass("correct horse")).unwrap();
        let want = alice_bundle();
        assert_eq!(back.swap_id, want.swap_id);
        assert_eq!(back.role, Role::Alice);
        assert_eq!(back.quote_json, want.quote_json);
        assert_eq!(back.maker_endpoint, want.maker_endpoint);
        assert_eq!(back.share.expose_secret(), want.share.expose_secret());
        assert_eq!(back.btc_key.expose_secret(), want.btc_key.expose_secret());
        assert_eq!(back.transcript, want.transcript);
        assert_eq!(back.blobs, want.blobs);
        assert_eq!(back.blob(ESIG_A_REFUND), want.blob(ESIG_A_REFUND));
    }

    #[test]
    fn spec_6_4_r1_alice_file_requires_her_own_refund_esig() {
        for required in ALICE_REQUIRED {
            let mut b = alice_bundle();
            b.blobs.retain(|(n, _)| n != required);
            assert!(
                matches!(seal_with(&b, fast("x")), Err(RecoveryError::Missing(m)) if m == required),
                "{required}"
            );
        }
    }

    #[test]
    fn wrong_passphrase_and_tampering_are_rejected() {
        let file = seal_with(&alice_bundle(), fast("right")).unwrap();
        assert!(matches!(
            open(&file, pass("wrong")),
            Err(RecoveryError::Decrypt)
        ));
        let mut damaged = file.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(matches!(
            open(&damaged, pass("right")),
            Err(RecoveryError::Decrypt)
        ));
    }

    #[test]
    fn file_does_not_leak_secrets_in_plaintext() {
        let mut b = alice_bundle();
        b.share = Share::from_bytes([0x5a; 32]);
        let file = seal_with(&b, fast("p")).unwrap();
        assert!(!file.windows(32).any(|w| w == [0x5a; 32]));
        assert!(!file.windows(9).any(|w| w == b"tx_refund"));
    }

    #[test]
    fn malformed_plaintext_is_rejected() {
        let good = encode(&alice_bundle()).unwrap();
        assert!(decode(&good).is_ok());
        for cut in [0, 5, 40, good.len() - 1] {
            assert!(
                matches!(decode(&good[..cut]), Err(RecoveryError::Format)),
                "cut {cut}"
            );
        }
        let mut trailing = good.to_vec();
        trailing.push(0);
        assert!(matches!(decode(&trailing), Err(RecoveryError::Format)));
        let mut bad_role = good.to_vec();
        bad_role[MAGIC.len() + 32] = 9;
        assert!(matches!(decode(&bad_role), Err(RecoveryError::Format)));
    }

    #[test]
    fn bob_file_needs_no_alice_fields() {
        let mut b = alice_bundle();
        b.role = Role::Bob;
        b.blobs.clear();
        let file = seal_with(&b, fast("b")).unwrap();
        assert_eq!(open(&file, pass("b")).unwrap().role, Role::Bob);
    }
}
