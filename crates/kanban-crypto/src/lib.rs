use std::path::Path;
use ed25519_dalek::{SigningKey, Signature, Signer, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Sha256, Digest};
use thiserror::Error;
use kanban_core::space::InviteMetadata;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid key")]
    InvalidKey,
    #[error("signature verification failed")]
    VerifyFailed,
    #[error("invalid base58 encoding")]
    InvalidBase58,
    #[error("invalid token length")]
    InvalidLength,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("invalid key format: {0}")]
    InvalidKeyFormat(String),
}

pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    pub fn generate() -> Self {
        Self { signing_key: SigningKey::generate(&mut OsRng) }
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }

    /// base32-encoded public key with `pk_` prefix (human-readable node ID)
    pub fn node_id(&self) -> String {
        let encoded = base32::encode(
            base32::Alphabet::RFC4648 { padding: false },
            &self.public_key_bytes(),
        ).to_lowercase();
        format!("pk_{encoded}")
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signing_key.sign(msg);
        sig.to_bytes().to_vec()
    }

    pub fn verify(pubkey_bytes: &[u8; 32], msg: &[u8], sig_bytes: &[u8]) -> Result<(), CryptoError> {
        let vk = VerifyingKey::from_bytes(pubkey_bytes).map_err(|_| CryptoError::InvalidKey)?;
        if vk.is_weak() {
            return Err(CryptoError::InvalidKey);
        }
        let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| CryptoError::InvalidKey)?;
        let sig = Signature::from_bytes(&sig_arr);
        vk.verify(msg, &sig).map_err(|_| CryptoError::VerifyFailed)
    }

    /// Export signing key bytes (for secure storage — caller is responsible for encryption)
    pub fn to_secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Restore from stored key bytes.
    ///
    /// # Safety
    ///
    /// `bytes` must be a valid previously-generated Ed25519 scalar. Passing
    /// zeroed or corrupted bytes produces a key whose derived public key will
    /// not match the original — always validate the round-trip after restore.
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        Self { signing_key: SigningKey::from_bytes(bytes) }
    }
}

pub fn generate_invite_token(space_id: &str, identity: &Identity) -> Result<String, CryptoError> {
    let uuid = uuid::Uuid::parse_str(space_id).map_err(|_| CryptoError::InvalidKey)?;
    let space_id_bytes = *uuid.as_bytes(); // [u8; 16]
    let pubkey_bytes = identity.public_key_bytes();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut payload = [0u8; 56];
    payload[0..16].copy_from_slice(&space_id_bytes);
    payload[16..48].copy_from_slice(&pubkey_bytes);
    payload[48..56].copy_from_slice(&timestamp.to_le_bytes());

    let sig = identity.sign(&payload);

    let mut token_bytes = [0u8; 120];
    token_bytes[0..56].copy_from_slice(&payload);
    token_bytes[56..120].copy_from_slice(&sig);

    Ok(bs58::encode(token_bytes).into_string())
}

pub fn verify_invite_token_signature(token: &str) -> Result<InviteMetadata, CryptoError> {
    let bytes = bs58::decode(token)
        .into_vec()
        .map_err(|_| CryptoError::InvalidBase58)?;

    if bytes.len() != 120 {
        return Err(CryptoError::InvalidLength);
    }

    // Compute token_hash from raw bytes (not from base58 string)
    let token_hash = hex::encode(Sha256::digest(&bytes));

    let space_id_bytes: [u8; 16] = bytes[0..16].try_into().unwrap();
    let pubkey_bytes: [u8; 32] = bytes[16..48].try_into().unwrap();
    let timestamp = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
    let sig_bytes = &bytes[56..120];

    // Verify signature over first 56 bytes
    Identity::verify(&pubkey_bytes, &bytes[0..56], sig_bytes)
        .map_err(|_| CryptoError::InvalidSignature)?;

    let space_id = uuid::Uuid::from_bytes(space_id_bytes).to_string();
    let owner_pubkey = hex::encode(pubkey_bytes);

    Ok(InviteMetadata { space_id, owner_pubkey, timestamp, token_hash })
}

pub fn import_ssh_identity(path: Option<&Path>) -> Result<Identity, CryptoError> {
    use ssh_key::PrivateKey;

    let key_path = match path {
        Some(p) => p.to_path_buf(),
        None => {
            let home = std::env::var("HOME").unwrap_or_default();
            std::path::PathBuf::from(home).join(".ssh").join("id_ed25519")
        }
    };

    if !key_path.exists() {
        return Err(CryptoError::FileNotFound(key_path.display().to_string()));
    }

    let pem = std::fs::read_to_string(&key_path)
        .map_err(|e| CryptoError::InvalidKeyFormat(e.to_string()))?;

    let private_key = PrivateKey::from_openssh(&pem)
        .map_err(|e| CryptoError::InvalidKeyFormat(e.to_string()))?;

    let ed25519_keypair = private_key
        .key_data()
        .ed25519()
        .ok_or_else(|| CryptoError::InvalidKeyFormat("not an Ed25519 key".into()))?;

    let secret_bytes: [u8; 32] = ed25519_keypair.private.to_bytes();
    Ok(Identity::from_secret_bytes(&secret_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity_has_stable_pubkey() {
        let id = Identity::generate();
        let pk1 = id.public_key_hex();
        let pk2 = id.public_key_hex();
        assert_eq!(pk1, pk2);
        assert_eq!(pk1.len(), 64); // 32 bytes hex-encoded
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = Identity::generate();
        let msg = b"hello world";
        let sig = id.sign(msg);
        let pk = id.public_key_bytes();
        Identity::verify(&pk, msg, &sig).unwrap();
    }

    #[test]
    fn verify_rejects_wrong_message() {
        let id = Identity::generate();
        let sig = id.sign(b"correct");
        let pk = id.public_key_bytes();
        assert!(Identity::verify(&pk, b"wrong", &sig).is_err());
    }
}

#[cfg(test)]
mod invite_tests {
    use super::*;

    #[test]
    fn generate_and_verify_token_roundtrip() {
        let identity = Identity::generate();
        let space_id = uuid::Uuid::new_v4().to_string();
        let token = generate_invite_token(&space_id, &identity).unwrap();
        let meta = verify_invite_token_signature(&token).unwrap();
        assert_eq!(meta.space_id, space_id);
        assert_eq!(meta.owner_pubkey, identity.public_key_hex());
        assert!(!meta.token_hash.is_empty());
    }

    #[test]
    fn verify_rejects_tampered_token() {
        let identity = Identity::generate();
        let space_id = uuid::Uuid::new_v4().to_string();
        let token = generate_invite_token(&space_id, &identity).unwrap();
        let mut bytes = bs58::decode(&token).into_vec().unwrap();
        bytes[60] ^= 0xFF; // flip bits in signature
        let tampered = bs58::encode(&bytes).into_string();
        assert!(matches!(
            verify_invite_token_signature(&tampered),
            Err(CryptoError::InvalidSignature)
        ));
    }

    #[test]
    fn verify_rejects_invalid_base58() {
        assert!(matches!(
            verify_invite_token_signature("not-valid-base58!!!"),
            Err(CryptoError::InvalidBase58)
        ));
    }

    #[test]
    fn verify_rejects_wrong_length() {
        let short = bs58::encode(vec![0u8; 50]).into_string();
        assert!(matches!(
            verify_invite_token_signature(&short),
            Err(CryptoError::InvalidLength)
        ));
    }
}
