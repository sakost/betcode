//! Shared database types and utilities.
//!
//! Provides `DatabaseError`, `unix_timestamp()`, and base64 encode/decode
//! used by both betcode-daemon and betcode-relay storage layers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Database errors shared across daemon and relay.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("I/O error: {0}")]
    Io(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("Query error: {0}")]
    Query(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

impl From<sqlx::Error> for DatabaseError {
    fn from(e: sqlx::Error) -> Self {
        DatabaseError::Query(e.to_string())
    }
}

/// Returns the current time as a Unix timestamp (seconds since epoch).
pub fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Simple base64 encoding (no external dependency needed).
pub fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;

        let _ = result.write_char(CHARS[(n >> 18 & 0x3F) as usize] as char);
        let _ = result.write_char(CHARS[(n >> 12 & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            let _ = result.write_char(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            let _ = result.write_char(CHARS[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

/// Simple base64 decoding for stored event payloads.
pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const DECODE: [u8; 128] = {
        let mut table = [255u8; 128];
        let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0;
        while i < 64 {
            table[chars[i] as usize] = i as u8;
            i += 1;
        }
        table
    };

    let input = input.trim_end_matches('=');
    if input.len() % 4 == 1 {
        return Err("Invalid base64 length".to_string());
    }
    let mut result = Vec::with_capacity(input.len() * 3 / 4);

    for chunk in input.as_bytes().chunks(4) {
        let mut n: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            if b as usize >= 128 || DECODE[b as usize] == 255 {
                return Err(format!("Invalid base64 character: {}", b as char));
            }
            n |= (DECODE[b as usize] as u32) << (18 - i * 6);
        }

        result.push((n >> 16 & 0xFF) as u8);
        if chunk.len() > 2 {
            result.push((n >> 8 & 0xFF) as u8);
        }
        if chunk.len() > 3 {
            result.push((n & 0xFF) as u8);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_base64() {
        let data = b"Hello, BetCode!";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn base64_empty() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn base64_padding() {
        // 1 byte → 4 chars with == padding
        let encoded = base64_encode(b"A");
        assert!(encoded.ends_with("=="));
        assert_eq!(base64_decode(&encoded).unwrap(), b"A");

        // 2 bytes → 4 chars with = padding
        let encoded = base64_encode(b"AB");
        assert!(encoded.ends_with('='));
        assert_eq!(base64_decode(&encoded).unwrap(), b"AB");
    }

    #[test]
    fn unix_timestamp_is_reasonable() {
        let ts = unix_timestamp();
        // Should be after 2024-01-01
        assert!(ts > 1_704_067_200);
    }
}
