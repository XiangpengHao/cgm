//! Crypto and framing primitives for the AiDEX DevComm2 protocol.
//!
//! A faithful port of `cgm.py` / `index.html`, validated against the device's
//! own captured vectors (see `PROTOCOL.md`). All transforms are derived from the
//! device serial, which is the shared secret.

use aes::Aes128;
use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use md5::{Digest, Md5};

/// Map a serial character to its base-36 value (`0-9`, `A-Z`/`a-z`).
///
/// Returns `None` for characters outside that set (the caller treats this as an
/// invalid serial).
pub fn base36(c: char) -> Option<u8> {
    match c {
        '0'..='9' => Some(c as u8 - b'0'),
        'A'..='Z' => Some(c as u8 - b'A' + 10),
        'a'..='z' => Some(c as u8 - b'a' + 10),
        _ => None,
    }
}

/// MD5 of `(base36(c) * mul + add) mod 256` over each serial character.
fn md5_transform(serial: &str, mul: u32, add: u32) -> Option<[u8; 16]> {
    let bytes: Option<Vec<u8>> = serial
        .chars()
        .map(|c| base36(c).map(|n| ((n as u32 * mul + add) & 0xff) as u8))
        .collect();
    Some(md5(&bytes?))
}

/// AES IV for a serial: `MD5((base36(c)*17 + 19) mod 256)`. Fixed per device.
pub fn derive_iv(serial: &str) -> Option<[u8; 16]> {
    md5_transform(serial, 17, 19)
}

/// The value written to characteristic `F001` during first-time pairing:
/// `MD5((base36(c)*13 + 61) mod 256)`.
pub fn derive_pair_secret(serial: &str) -> Option<[u8; 16]> {
    md5_transform(serial, 13, 61)
}

/// MD5 digest of `data`.
pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(data);
    h.finalize().into()
}

/// Single AES-128 ECB block encryption (the keystream primitive for CFB).
fn aes_encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut b = GenericArray::clone_from_slice(block);
    cipher.encrypt_block(&mut b);
    b.into()
}

/// AES-128-CFB with **128-bit (full-block) feedback**, matching CommonCrypto
/// `kCCModeCFB`. Implemented by hand so a partial final block (e.g. the 17-byte
/// reconnect challenge) is byte-exact. The IV is *not* chained across calls.
pub fn cfb128(key: &[u8; 16], iv: &[u8; 16], data: &[u8], decrypt: bool) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    let mut feedback = *iv;
    let mut off = 0;
    while off < data.len() {
        let keystream = aes_encrypt_block(key, &feedback);
        let n = core::cmp::min(16, data.len() - off);
        for i in 0..n {
            out[off + i] = data[off + i] ^ keystream[i];
        }
        // CFB feeds the ciphertext block forward (= input when decrypting,
        // output when encrypting); pad the trailing partial block with zeros.
        let mut next = [0u8; 16];
        for (i, slot) in next.iter_mut().enumerate() {
            if off + i < data.len() {
                *slot = if decrypt { data[off + i] } else { out[off + i] };
            }
        }
        feedback = next;
        off += 16;
    }
    out
}

/// CRC-16/CCITT-FALSE: poly `0x1021`, init `0xFFFF`, no reflection, no final XOR.
pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// CRC-8/MAXIM (Dallas): reflected poly `0x8C`, init `0x00`.
pub fn crc8_maxim(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8c
            } else {
                crc >> 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // Worked example from PROTOCOL.md (fabricated, self-consistent vectors).
    const SERIAL: &str = "EXAMPLE001";
    const IV: &str = "8ADF6CA259158051F483161F11CC1D6F";
    const PAIR_KEY: &str = "0123456789ABCDEF0123456789ABCDEF";
    const CHALLENGE: &str = "2E390C058040DF1996C4AEA2CB16274ED2";
    const SESSION_KEY: &str = "00112233445566778899AABBCCDDEEFF";

    #[test]
    fn iv_matches_protocol_vector() {
        let iv = derive_iv(SERIAL).unwrap();
        assert_eq!(iv.to_vec(), hex(IV));
    }

    #[test]
    fn reconnect_challenge_decrypts_to_session_key() {
        let iv = derive_iv(SERIAL).unwrap();
        let key: [u8; 16] = hex(PAIR_KEY).try_into().unwrap();
        let plain = cfb128(&key, &iv, &hex(CHALLENGE), true);
        assert_eq!(&plain[..16], hex(SESSION_KEY).as_slice());
        assert_eq!(crc8_maxim(&plain[..16]), plain[16]);
        assert_eq!(plain[16], 0xd7);
    }

    #[test]
    fn cfb_round_trips() {
        let key: [u8; 16] = hex(PAIR_KEY).try_into().unwrap();
        let iv = derive_iv(SERIAL).unwrap();
        let msg = b"hello devcomm2 packet!"; // 22 bytes -> partial final block
        let ct = cfb128(&key, &iv, msg, false);
        let pt = cfb128(&key, &iv, &ct, true);
        assert_eq!(&pt, msg);
    }

    #[test]
    fn crc_known_values() {
        // CRC16-CCITT-FALSE("123456789") = 0x29B1; CRC8-MAXIM = 0xA1.
        assert_eq!(crc16_ccitt(b"123456789"), 0x29b1);
        assert_eq!(crc8_maxim(b"123456789"), 0xa1);
    }

    #[test]
    fn base36_mapping() {
        assert_eq!(base36('0'), Some(0));
        assert_eq!(base36('9'), Some(9));
        assert_eq!(base36('A'), Some(10));
        assert_eq!(base36('Z'), Some(35));
        assert_eq!(base36('a'), Some(10));
        assert_eq!(base36('-'), None);
    }
}
