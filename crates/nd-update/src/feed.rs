//! The signed feed: `releases.<channel>.json` as `vpk pack` writes it, plus
//! `releases.<channel>.json.sig` holding a base64 ed25519 signature.
//!
//! The signed message binds the channel name to the bytes, so a host cannot serve the beta feed
//! under the stable name. Package hashes inside the feed are what Velopack checks after each
//! download, which makes the signature cover the packages too.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

const DOMAIN: &[u8] = b"nd-feed-v1\0";

/// `releases.<channel>.json`.
pub fn feed_file(channel: &str) -> String {
    format!("releases.{channel}.json")
}

/// `releases.<channel>.json.sig`.
pub fn signature_file(channel: &str) -> String {
    format!("releases.{channel}.json.sig")
}

#[derive(Debug, PartialEq, Eq)]
pub enum FeedError {
    /// The key text is not base64 of 32 bytes.
    BadKey,
    /// The signature file is not base64 of 64 bytes.
    BadSignature,
    /// Well-formed signature that does not match this feed, channel and key.
    Mismatch,
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadKey => "update key is not base64 ed25519 (32 bytes)",
            Self::BadSignature => "feed signature is not base64 ed25519 (64 bytes)",
            Self::Mismatch => "feed signature does not match: the feed was changed or signed with another key",
        })
    }
}

impl std::error::Error for FeedError {}

fn message(channel: &str, feed: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(DOMAIN.len() + channel.len() + 1 + feed.len());
    msg.extend_from_slice(DOMAIN);
    msg.extend_from_slice(channel.as_bytes());
    msg.push(0);
    msg.extend_from_slice(feed);
    msg
}

fn decode<const N: usize>(text: &str, err: FeedError) -> Result<[u8; N], FeedError> {
    let bytes = STANDARD.decode(text.trim()).map_err(|_| FeedError::BadKey)?;
    bytes.try_into().map_err(|_| err)
}

/// The public half, shipped in the app.
#[derive(Clone, Debug)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    pub fn parse(text: &str) -> Result<Self, FeedError> {
        let bytes = decode::<32>(text, FeedError::BadKey)?;
        VerifyingKey::from_bytes(&bytes).map(Self).map_err(|_| FeedError::BadKey)
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0.to_bytes())
    }

    /// Checks `signature` (the `.sig` file's text) over `feed` for `channel`.
    pub fn verify(&self, channel: &str, feed: &[u8], signature: &str) -> Result<(), FeedError> {
        let bytes = STANDARD.decode(signature.trim()).map_err(|_| FeedError::BadSignature)?;
        let bytes: [u8; 64] = bytes.try_into().map_err(|_| FeedError::BadSignature)?;
        let signature = Signature::from_bytes(&bytes);
        self.0.verify_strict(&message(channel, feed), &signature).map_err(|_| FeedError::Mismatch)
    }
}

/// The private half, kept by whoever runs `nd release`.
pub struct SecretKey(SigningKey);

impl SecretKey {
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("the OS random source is available");
        Self(SigningKey::from_bytes(&seed))
    }

    /// Base64 of the 32-byte seed.
    pub fn parse(text: &str) -> Result<Self, FeedError> {
        decode::<32>(text, FeedError::BadKey).map(|seed| Self(SigningKey::from_bytes(&seed)))
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0.to_bytes())
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }

    /// The `.sig` file's text for `feed` on `channel`.
    pub fn sign(&self, channel: &str, feed: &[u8]) -> String {
        let mut text = STANDARD.encode(self.0.sign(&message(channel, feed)).to_bytes());
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_round_trips_and_binds_bytes_channel_and_key() {
        let key = SecretKey::generate();
        let public = PublicKey::parse(&key.public_key().to_base64()).unwrap();
        let feed = br#"{"Assets":[]}"#;
        let sig = key.sign("stable", feed);
        assert_eq!(public.verify("stable", feed, &sig), Ok(()));
        assert_eq!(public.verify("beta", feed, &sig), Err(FeedError::Mismatch));
        assert_eq!(public.verify("stable", br#"{"Assets":[{}]}"#, &sig), Err(FeedError::Mismatch));
        let other = SecretKey::generate().public_key();
        assert_eq!(other.verify("stable", feed, &sig), Err(FeedError::Mismatch));
        assert_eq!(public.verify("stable", feed, "not base64!"), Err(FeedError::BadSignature));
        assert_eq!(public.verify("stable", feed, &STANDARD.encode([0u8; 12])), Err(FeedError::BadSignature));
    }

    #[test]
    fn keys_round_trip_through_base64() {
        let key = SecretKey::generate();
        let again = SecretKey::parse(&key.to_base64()).unwrap();
        assert_eq!(key.public_key().to_base64(), again.public_key().to_base64());
        assert!(PublicKey::parse("short").is_err());
        assert!(SecretKey::parse(&STANDARD.encode([1u8; 31])).is_err());
    }
}
