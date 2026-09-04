//! Cryptographic primitives for `ctx-core`.
//!
//! This module provides:
//! - X25519 keypair generation for `age` encryption.
//! - Age-based public key encryption and secret key decryption.
//! - Password-based key derivation using Argon2id.
//! - SHA-256 cryptographic hashing returning lowercase hex digests.

use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};
use argon2::Argon2;
use sha2::{Digest, Sha256};

use crate::error::{CtxError, Result};

/// Hexadecimal character lookup table for encoding.
const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Encodes a byte slice into a lowercase hexadecimal string.
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(HEX_CHARS[(byte >> 4) as usize] as char);
        hex.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    hex
}

/// Generates a new age X25519 keypair.
///
/// Returns a tuple of `(public_key, secret_key)` formatted as Bech32 strings:
/// - The public key begins with `age1...`
/// - The secret key begins with `AGE-SECRET-KEY-1...`
///
/// # Examples
///
/// ```
/// use ctx_core::crypto::generate_keypair;
///
/// let (public_key, secret_key) = generate_keypair();
/// assert!(public_key.starts_with("age1"));
/// assert!(secret_key.starts_with("AGE-SECRET-KEY-1"));
/// ```
pub fn generate_keypair() -> (String, String) {
    let identity = Identity::generate();
    let public_key = identity.to_public().to_string();
    let secret_key = identity.to_string().expose_secret().to_string();
    (public_key, secret_key)
}

/// Encrypts raw data bytes using the recipient's age X25519 public key.
///
/// # Arguments
///
/// * `public_key` - Bech32-encoded age public key (starts with `age1`).
/// * `data` - Plaintext bytes to encrypt.
///
/// # Errors
///
/// Returns [`CtxError::Crypto`] if the public key cannot be parsed or encryption fails.
///
/// # Examples
///
/// ```
/// use ctx_core::crypto::{generate_keypair, encrypt_bytes, decrypt_bytes};
///
/// let (pub_key, sec_key) = generate_keypair();
/// let plaintext = b"sensitive payload";
/// let ciphertext = encrypt_bytes(&pub_key, plaintext).expect("encryption succeeds");
/// let decrypted = decrypt_bytes(&sec_key, &ciphertext).expect("decryption succeeds");
/// assert_eq!(plaintext, &decrypted[..]);
/// ```
pub fn encrypt_bytes(public_key: &str, data: &[u8]) -> Result<Vec<u8>> {
    let recipient: Recipient = public_key
        .trim()
        .parse()
        .map_err(|e| CtxError::crypto(format!("Invalid age public key: {e}")))?;

    age::encrypt(&recipient, data).map_err(|e| CtxError::crypto(format!("Encryption failed: {e}")))
}

/// Decrypts ciphertext bytes using the recipient's age X25519 secret key.
///
/// # Arguments
///
/// * `secret_key` - Bech32-encoded age secret key (starts with `AGE-SECRET-KEY-1`).
/// * `data` - Age ciphertext bytes (binary or ASCII armored).
///
/// # Errors
///
/// Returns [`CtxError::Crypto`] if the secret key cannot be parsed or decryption fails.
///
/// # Examples
///
/// ```
/// use ctx_core::crypto::{generate_keypair, encrypt_bytes, decrypt_bytes};
///
/// let (pub_key, sec_key) = generate_keypair();
/// let ciphertext = encrypt_bytes(&pub_key, b"secret data").expect("encryption succeeds");
/// let decrypted = decrypt_bytes(&sec_key, &ciphertext).expect("decryption succeeds");
/// assert_eq!(b"secret data", &decrypted[..]);
/// ```
pub fn decrypt_bytes(secret_key: &str, data: &[u8]) -> Result<Vec<u8>> {
    let identity: Identity = secret_key
        .trim()
        .parse()
        .map_err(|e| CtxError::crypto(format!("Invalid age secret key: {e}")))?;

    age::decrypt(&identity, data).map_err(|e| CtxError::crypto(format!("Decryption failed: {e}")))
}

/// Derives a 256-bit cryptographic key from a password and salt using Argon2id.
///
/// Returns the derived key formatted as a 64-character lowercase hexadecimal string.
///
/// # Arguments
///
/// * `password` - The master password or passphrase to derive from.
/// * `salt` - Cryptographic salt (recommended minimum 8 bytes, typically 16 bytes).
///
/// # Errors
///
/// Returns [`CtxError::Crypto`] if Argon2id key derivation fails (e.g. salt is shorter than 8 bytes).
///
/// # Examples
///
/// ```
/// use ctx_core::crypto::derive_key_from_password;
///
/// let key = derive_key_from_password("mypassword", b"0123456789abcdef")
///     .expect("derivation succeeds");
/// assert_eq!(key.len(), 64);
/// ```
pub fn derive_key_from_password(password: &str, salt: &[u8]) -> Result<String> {
    let mut key_bytes = [0u8; 32];
    let argon2 = Argon2::default();

    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key_bytes)
        .map_err(|e| CtxError::crypto(format!("Key derivation failed: {e}")))?;

    let hex_key = bytes_to_hex(&key_bytes);
    // Overwrite sensitive key material in stack buffer
    key_bytes.fill(0);

    Ok(hex_key)
}

/// Computes the SHA-256 cryptographic hash of the input data and returns a lowercase hex digest.
///
/// # Arguments
///
/// * `data` - Raw bytes to hash.
///
/// # Examples
///
/// ```
/// use ctx_core::crypto::hash_sha256;
///
/// let digest = hash_sha256(b"hello world");
/// assert_eq!(digest, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
/// ```
pub fn hash_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    bytes_to_hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_keypair_format_and_uniqueness() {
        let (pub1, sec1) = generate_keypair();
        let (pub2, sec2) = generate_keypair();

        assert!(pub1.starts_with("age1"));
        assert!(pub2.starts_with("age1"));
        assert!(sec1.starts_with("AGE-SECRET-KEY-1"));
        assert!(sec2.starts_with("AGE-SECRET-KEY-1"));

        assert_ne!(pub1, pub2);
        assert_ne!(sec1, sec2);

        // Verify that generated keys can be parsed back
        assert!(pub1.parse::<Recipient>().is_ok());
        assert!(sec1.parse::<Identity>().is_ok());
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let (pub_key, sec_key) = generate_keypair();
        let message = b"Hello, secure context synchronization across machines!";

        let ciphertext = encrypt_bytes(&pub_key, message).expect("encrypt_bytes failed");
        assert_ne!(ciphertext, message);

        let decrypted = decrypt_bytes(&sec_key, &ciphertext).expect("decrypt_bytes failed");
        assert_eq!(decrypted, message);
    }

    #[test]
    fn test_encrypt_decrypt_empty_payload() {
        let (pub_key, sec_key) = generate_keypair();
        let message = b"";

        let ciphertext = encrypt_bytes(&pub_key, message).expect("encrypt_bytes failed on empty");
        let decrypted = decrypt_bytes(&sec_key, &ciphertext).expect("decrypt_bytes failed on empty");
        assert_eq!(decrypted, message);
    }

    #[test]
    fn test_encrypt_decrypt_large_payload() {
        let (pub_key, sec_key) = generate_keypair();
        let large_data: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();

        let ciphertext = encrypt_bytes(&pub_key, &large_data).expect("encrypt_bytes failed on large data");
        let decrypted = decrypt_bytes(&sec_key, &ciphertext).expect("decrypt_bytes failed on large data");
        assert_eq!(decrypted, large_data);
    }

    #[test]
    fn test_decrypt_with_wrong_key() {
        let (pub_key1, _sec_key1) = generate_keypair();
        let (_pub_key2, sec_key2) = generate_keypair();

        let message = b"Super confidential payload";
        let ciphertext = encrypt_bytes(&pub_key1, message).expect("encryption succeeds");

        let result = decrypt_bytes(&sec_key2, &ciphertext);
        assert!(result.is_err());
        let err = result.expect_err("should fail with wrong secret key");
        match err {
            CtxError::Crypto(msg) => {
                assert!(msg.contains("Decryption failed"));
            }
            other => panic!("Expected CtxError::Crypto, got {:?}", other),
        }
    }

    #[test]
    fn test_encrypt_invalid_public_key() {
        let result = encrypt_bytes("not-a-valid-key", b"test data");
        assert!(result.is_err());
        let err = result.expect_err("should fail");
        match err {
            CtxError::Crypto(msg) => {
                assert!(msg.contains("Invalid age public key"));
            }
            other => panic!("Expected CtxError::Crypto, got {:?}", other),
        }
    }

    #[test]
    fn test_decrypt_invalid_secret_key() {
        let result = decrypt_bytes("not-a-valid-secret", b"test data");
        assert!(result.is_err());
        let err = result.expect_err("should fail");
        match err {
            CtxError::Crypto(msg) => {
                assert!(msg.contains("Invalid age secret key"));
            }
            other => panic!("Expected CtxError::Crypto, got {:?}", other),
        }
    }

    #[test]
    fn test_decrypt_corrupted_ciphertext() {
        let (pub_key, sec_key) = generate_keypair();
        let message = b"Payload that will be corrupted";
        let mut ciphertext = encrypt_bytes(&pub_key, message).expect("encryption succeeds");

        // Corrupt the ciphertext by tampering with the last bytes (payload MAC / data)
        let len = ciphertext.len();
        if len > 5 {
            ciphertext[len - 1] ^= 0xff;
            ciphertext[len - 2] ^= 0xff;
        }

        let result = decrypt_bytes(&sec_key, &ciphertext);
        assert!(result.is_err());
        match result.expect_err("should fail on corrupted ciphertext") {
            CtxError::Crypto(msg) => {
                assert!(msg.contains("Decryption failed"));
            }
            other => panic!("Expected CtxError::Crypto, got {:?}", other),
        }
    }

    #[test]
    fn test_derive_key_from_password_deterministic() {
        let password = "my-secure-password-123";
        let salt = b"salt_for_testing_16";

        let key1 = derive_key_from_password(password, salt).expect("derivation 1 succeeds");
        let key2 = derive_key_from_password(password, salt).expect("derivation 2 succeeds");

        assert_eq!(key1, key2);
        // 32 bytes encoded as hex must be 64 characters
        assert_eq!(key1.len(), 64);
        assert!(key1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_derive_key_from_password_variations() {
        let salt = b"salt_for_testing_16";

        let key_a = derive_key_from_password("password_a", salt).expect("derivation succeeds");
        let key_b = derive_key_from_password("password_b", salt).expect("derivation succeeds");
        assert_ne!(key_a, key_b);

        let salt2 = b"different_salt_16b";
        let key_c = derive_key_from_password("password_a", salt2).expect("derivation succeeds");
        assert_ne!(key_a, key_c);
    }

    #[test]
    fn test_derive_key_from_password_salt_too_short() {
        let result = derive_key_from_password("password", b"short");
        assert!(result.is_err());
        match result.expect_err("should fail when salt < 8 bytes") {
            CtxError::Crypto(msg) => {
                assert!(msg.contains("Key derivation failed"));
            }
            other => panic!("Expected CtxError::Crypto, got {:?}", other),
        }
    }

    #[test]
    fn test_hash_sha256_known_vectors() {
        // SHA-256("")
        let empty_hash = hash_sha256(b"");
        assert_eq!(
            empty_hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        // SHA-256("hello world")
        let hello_hash = hash_sha256(b"hello world");
        assert_eq!(
            hello_hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );

        // SHA-256("ctx")
        let ctx_hash = hash_sha256(b"ctx");
        assert_eq!(
            ctx_hash,
            "0230c6b1d833c51cc426492022677b74c60d82891931221a42db9e7bb06205e9"
        );
    }
}
