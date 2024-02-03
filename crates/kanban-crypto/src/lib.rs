use ed25519_dalek::{SigningKey, Signature, Signer, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid key")]
    InvalidKey,
    #[error("signature verification failed")]
    VerifyFailed,
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
        let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| CryptoError::InvalidKey)?;
        let sig = Signature::from_bytes(&sig_arr);
        vk.verify(msg, &sig).map_err(|_| CryptoError::VerifyFailed)
    }

    /// Export signing key bytes (for secure storage — caller is responsible for encryption)
    pub fn to_secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Restore from stored key bytes
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        Self { signing_key: SigningKey::from_bytes(bytes) }
    }
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
