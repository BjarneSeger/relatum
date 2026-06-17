//! A user's visual signature.
//!
//! A [`Signature`] is the image a trainee or signer registers once and that a
//! future PDF export stamps onto their reports — next to the author for the
//! trainee, next to the sign-off for the signer. It is a value object: a validated
//! bag of image bytes plus the format they are in. The domain treats it as inert
//! content; *how* it is captured (drawn on a canvas, uploaded as a file) and *how*
//! it is rendered into a PDF live in the outer layers.
//!
//! This is the first binary payload in the otherwise text-only domain, so
//! [`Signature::new`] is the single gate that keeps junk out: it rejects an empty
//! image, anything larger than [`Signature::MAX_BYTES`], and bytes that are not the
//! declared format (today only PNG, recognised by its magic number).

use crate::DomainError;
use jiff::Timestamp;

/// The image format of a [`Signature`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureFormat {
    /// A PNG raster image.
    Png,
}

impl SignatureFormat {
    /// The canonical lowercase token this format is persisted and sent as — it
    /// matches the `signatures.format` CHECK and the API DTO.
    pub fn as_str(&self) -> &'static str {
        match self {
            SignatureFormat::Png => "png",
        }
    }

    /// Parse the canonical token back into a format.
    pub fn parse(token: &str) -> Result<Self, DomainError> {
        match token {
            "png" => Ok(SignatureFormat::Png),
            other => Err(DomainError::Invalid(format!(
                "unsupported signature format {other:?}"
            ))),
        }
    }
}

/// A validated signature image: its [format](SignatureFormat) and raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    format: SignatureFormat,
    bytes: Vec<u8>,
}

impl Signature {
    /// The largest signature image accepted, in bytes (256 KiB). A hand-drawn or
    /// scanned signature is comfortably smaller; the cap bounds both storage and the
    /// request body the API must buffer before this check can even run.
    pub const MAX_BYTES: usize = 256 * 1024;

    /// The 8-byte signature every PNG file begins with.
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    /// Validate `bytes` as an image of `format` and wrap them.
    ///
    /// Fails with [`DomainError::Invalid`] if the image is empty, exceeds
    /// [`MAX_BYTES`](Self::MAX_BYTES), or does not begin with the magic number of
    /// the declared `format`.
    pub fn new(format: SignatureFormat, bytes: Vec<u8>) -> Result<Self, DomainError> {
        if bytes.is_empty() {
            return Err(DomainError::Invalid("signature image is empty".into()));
        }
        if bytes.len() > Self::MAX_BYTES {
            return Err(DomainError::Invalid(format!(
                "signature image is {} bytes, over the {}-byte limit",
                bytes.len(),
                Self::MAX_BYTES
            )));
        }
        match format {
            SignatureFormat::Png if !bytes.starts_with(&Self::PNG_MAGIC) => {
                return Err(DomainError::Invalid(
                    "signature image is not a valid PNG".into(),
                ));
            }
            SignatureFormat::Png => {}
        }
        Ok(Self { format, bytes })
    }

    /// Reconstitute a signature from already-stored parts, skipping validation.
    ///
    /// The storage adapter uses this to rebuild a row it previously wrote through
    /// [`new`](Self::new) — the bytes were validated then. It is deliberately *not* a
    /// way for application logic to bypass validation; build fresh signatures with
    /// [`new`](Self::new).
    pub fn from_stored(format: SignatureFormat, bytes: Vec<u8>) -> Self {
        Self { format, bytes }
    }

    /// The image format.
    pub fn format(&self) -> SignatureFormat {
        self.format
    }

    /// The raw image bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A [`Signature`] as held in storage, paired with the instant it was last set.
///
/// The read model the [`SignatureStorage`](crate::ports::signaturestorage::SignatureStorage)
/// port hands back: the image plus an audit timestamp a future PDF export can caption.
#[derive(Debug, Clone)]
pub struct StoredSignature {
    /// The stored signature image.
    pub signature: Signature,
    /// When the signature was last set, in UTC.
    pub updated_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG header followed by a couple of bytes — enough to pass the magic-number
    /// check (we validate the signature, not the full image).
    fn png_bytes() -> Vec<u8> {
        vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01]
    }

    #[test]
    fn accepts_a_well_formed_png() {
        let sig = Signature::new(SignatureFormat::Png, png_bytes()).unwrap();
        assert_eq!(sig.format(), SignatureFormat::Png);
        assert_eq!(sig.bytes(), png_bytes().as_slice());
    }

    #[test]
    fn rejects_an_empty_image() {
        assert!(matches!(
            Signature::new(SignatureFormat::Png, Vec::new()),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_an_oversized_image() {
        let mut bytes = png_bytes();
        bytes.resize(Signature::MAX_BYTES + 1, 0);
        assert!(matches!(
            Signature::new(SignatureFormat::Png, bytes),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_non_png_bytes() {
        assert!(matches!(
            Signature::new(SignatureFormat::Png, b"not a png".to_vec()),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn format_token_round_trips() {
        assert_eq!(SignatureFormat::Png.as_str(), "png");
        assert_eq!(SignatureFormat::parse("png").unwrap(), SignatureFormat::Png);
        assert!(matches!(
            SignatureFormat::parse("gif"),
            Err(DomainError::Invalid(_))
        ));
    }
}
