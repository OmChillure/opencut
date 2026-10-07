//! Password check for local sign-in. The stored form is
//! `pbkdf2-sha256$<iters>$<salt hex>$<hash hex>`.

use hmac::Mac;
use sha2::Sha256;

pub const PBKDF2_ITERS: u32 = 100_000;

type HmacSha256 = hmac::Hmac<Sha256>;

pub fn hash_password(password: &str, salt: &[u8], iters: u32) -> String {
    let derived = pbkdf2_sha256(password.as_bytes(), salt, iters, 32);
    format!(
        "pbkdf2-sha256${iters}${}${}",
        hex_encode(salt),
        hex_encode(&derived)
    )
}

pub fn password_matches(password: &str, stored: &str) -> bool {
    let mut parts = stored.split('$');
    if parts.next() != Some("pbkdf2-sha256") {
        return false;
    }
    let Some(iters) = parts.next().and_then(|raw| raw.parse::<u32>().ok()) else {
        return false;
    };
    if iters == 0 || iters > 2_000_000 {
        return false;
    }
    let Some(salt) = parts.next().and_then(hex_decode) else {
        return false;
    };
    let Some(want) = parts.next().and_then(hex_decode) else {
        return false;
    };
    if parts.next().is_some() || salt.is_empty() || want.is_empty() {
        return false;
    }
    let got = pbkdf2_sha256(password.as_bytes(), &salt, iters, want.len());
    same_bytes(&got, &want)
}

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iters: u32, dk_len: usize) -> Vec<u8> {
    let iters = iters.max(1);
    let mut out = vec![0u8; dk_len];
    let mut block = 1u32;
    let mut offset = 0;
    while offset < dk_len {
        let mut mac = HmacSha256::new_from_slice(password).expect("hmac-sha256 takes any key");
        mac.update(salt);
        mac.update(&block.to_be_bytes());
        let mut rolling = mac.finalize().into_bytes();
        let mut mixed = rolling.clone();
        for _ in 1..iters {
            let mut mac = HmacSha256::new_from_slice(password).expect("hmac-sha256 takes any key");
            mac.update(&rolling);
            rolling = mac.finalize().into_bytes();
            for (acc, byte) in mixed.iter_mut().zip(rolling.iter()) {
                *acc ^= *byte;
            }
        }
        let take = (dk_len - offset).min(mixed.len());
        out[offset..offset + take].copy_from_slice(&mixed[..take]);
        offset += take;
        block = block.saturating_add(1);
    }
    out
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(raw: &str) -> Option<Vec<u8>> {
    if raw.is_empty() || raw.len() % 2 != 0 {
        return None;
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let hi = hex_val(bytes[index])?;
        let lo = hex_val(bytes[index + 1])?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    Some(out)
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn same_bytes(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_the_sha256_vector() {
        // RFC 6070-style vector for HMAC-SHA256, c = 1 and c = 2, dkLen = 32.
        let once = pbkdf2_sha256(b"password", b"salt", 1, 32);
        assert_eq!(
            hex_encode(&once),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        let twice = pbkdf2_sha256(b"password", b"salt", 2, 32);
        assert_eq!(
            hex_encode(&twice),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
    }

    #[test]
    fn a_stored_password_matches_only_itself() {
        let stored = hash_password("correct-horse", b"saltsaltsalt!!!!", 1);
        assert!(password_matches("correct-horse", &stored));
        assert!(!password_matches("wrong-horse", &stored));
        assert!(!password_matches("correct-horse", "plain"));
        assert!(!password_matches("correct-horse", "pbkdf2-sha256$0$aa$bb"));
        assert!(stored.starts_with("pbkdf2-sha256$1$"));
    }
}
