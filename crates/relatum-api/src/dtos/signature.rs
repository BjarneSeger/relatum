//! Signature-capture DTOs.
//!
//! A user registers one signature image, carried as **base64 inside JSON** — never a
//! raw binary body — so the generated `relatum-client` stays typed (progenitor does
//! not handle binary request/response bodies well). The format is a small enum
//! mirroring [`MarkerDto`](super::user::MarkerDto).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use relatum_domain::models::signature::{SignatureFormat, StoredSignature};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The image format of a signature, on the wire.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignatureFormatDto {
    Png,
}

impl From<SignatureFormat> for SignatureFormatDto {
    fn from(format: SignatureFormat) -> Self {
        match format {
            SignatureFormat::Png => SignatureFormatDto::Png,
        }
    }
}

impl From<SignatureFormatDto> for SignatureFormat {
    fn from(format: SignatureFormatDto) -> Self {
        match format {
            SignatureFormatDto::Png => SignatureFormat::Png,
        }
    }
}

/// Body for setting (or replacing) the caller's own signature.
///
/// `data_base64` is the standard-base64 encoding of the raw image bytes. It is a
/// plain string on purpose: a raw binary body would not survive OpenAPI → client
/// generation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetSignatureRequest {
    /// The image format of the encoded bytes.
    pub format: SignatureFormatDto,
    /// Standard-base64 of the raw image bytes.
    #[schema(example = "iVBORw0KGgoAAAANSUhEUgAA...")]
    pub data_base64: String,
}

impl SetSignatureRequest {
    /// Decode the base64 payload into raw image bytes.
    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        BASE64.decode(self.data_base64.as_bytes())
    }
}

/// The caller's stored signature, returned by `GET /api/v1/me/signature`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SignatureView {
    /// The image format.
    pub format: SignatureFormatDto,
    /// Standard-base64 of the raw image bytes.
    pub data_base64: String,
    /// When the signature was last set, RFC 3339.
    #[schema(example = "2026-06-14T09:30:00Z")]
    pub updated_at: String,
}

impl From<&StoredSignature> for SignatureView {
    fn from(stored: &StoredSignature) -> Self {
        SignatureView {
            format: stored.signature.format().into(),
            data_base64: BASE64.encode(stored.signature.bytes()),
            updated_at: stored.updated_at.to_string(),
        }
    }
}
