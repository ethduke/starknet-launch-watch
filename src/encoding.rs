use anyhow::{Result, bail, ensure};
use num_bigint::BigUint;
use sha3::{Digest, Keccak256};

pub fn number(s: &str) -> Result<BigUint> {
    ensure!(s.len() <= 80, "numeric value exceeds limit");
    let n = if let Some(hex) = s.strip_prefix("0x") {
        BigUint::parse_bytes(hex.as_bytes(), 16)
    } else {
        BigUint::parse_bytes(s.as_bytes(), 10)
    };
    n.ok_or_else(|| anyhow::anyhow!("invalid numeric value"))
}

pub fn felt(s: &str) -> Result<String> {
    let n = number(s)?;
    let modulus =
        (BigUint::from(1u8) << 251usize) + (BigUint::from(17u8) << 192usize) + BigUint::from(1u8);
    ensure!(n < modulus, "value is outside Starknet field");
    Ok(format!("0x{n:x}"))
}

pub fn limb(s: &str) -> Result<BigUint> {
    let n = number(s)?;
    ensure!(n.bits() <= 128, "invalid u128 limb");
    Ok(n)
}

pub fn uint256(data: &[String]) -> Result<String> {
    ensure!(data.len() == 2, "invalid u256 length");
    Ok((limb(&data[0])? + (limb(&data[1])? << 128usize)).to_string())
}

pub fn selector(name: &str) -> String {
    let mut bytes = Keccak256::digest(name.as_bytes());
    bytes[0] &= 3;
    format!("0x{:x}", BigUint::from_bytes_be(&bytes))
}

pub fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c,
        '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200b}'..='\u{200f}')
        })
        .take(80)
        .collect()
}

pub fn cairo_text(data: &[String]) -> Result<String> {
    ensure!(
        !data.is_empty() && data.len() <= 64,
        "invalid metadata length"
    );
    if data.len() == 1 {
        let b = number(&data[0])?.to_bytes_be();
        ensure!(b.len() <= 31, "invalid short string");
        return Ok(clean_text(std::str::from_utf8(&b)?));
    }
    // Cairo ByteArray: full-word count, 31-byte words, pending word, pending length.
    let n: usize = number(&data[0])?.to_string().parse()?;
    ensure!(n <= 60 && data.len() == n + 3, "invalid ByteArray layout");
    let mut bytes = Vec::new();
    for word in &data[1..=n] {
        let b = number(word)?.to_bytes_be();
        ensure!(b.len() <= 31, "invalid ByteArray word");
        bytes.extend(std::iter::repeat_n(0, 31 - b.len()));
        bytes.extend(b);
    }
    let pending_len: usize = number(&data[n + 2])?.to_string().parse()?;
    ensure!(pending_len < 31, "invalid pending length");
    let b = number(&data[n + 1])?.to_bytes_be();
    if pending_len == 0 {
        ensure!(
            number(&data[n + 1])? == BigUint::from(0u8),
            "nonempty pending word"
        );
    } else {
        ensure!(b.len() <= pending_len, "invalid pending word");
        bytes.extend(std::iter::repeat_n(0, pending_len - b.len()));
        bytes.extend(b);
    }
    Ok(clean_text(std::str::from_utf8(&bytes)?))
}

pub fn display_units(raw: &str, decimals: u32) -> Result<String> {
    ensure!(decimals <= 36, "unsupported decimal count");
    let s = number(raw)?.to_string();
    if decimals == 0 {
        return Ok(s);
    }
    let padded = format!("{:0>width$}", s, width = decimals as usize + 1);
    let cut = padded.len() - decimals as usize;
    let fraction = padded[cut..].trim_end_matches('0');
    Ok(if fraction.is_empty() {
        padded[..cut].to_owned()
    } else {
        format!("{}.{}", &padded[..cut], fraction)
    })
}

pub fn require_len(data: &[String], expected: usize) -> Result<()> {
    if data.len() != expected {
        bail!(
            "event layout mismatch: expected {expected} values, got {}",
            data.len()
        );
    }
    for value in data {
        felt(value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selectors_use_keccak_not_sha3() {
        assert_eq!(
            selector("transfer"),
            "0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e"
        );
    }
    #[test]
    fn normalization_and_field_limits() {
        assert_eq!(felt("0x000AbC").unwrap(), "0xabc");
        assert!(felt(&format!("0x{}", "f".repeat(64))).is_err());
        assert!(felt("https://secret.invalid").is_err());
    }
    #[test]
    fn full_width_supply() {
        assert_eq!(
            uint256(&["0x0".into(), "0x1".into()]).unwrap(),
            "340282366920938463463374607431768211456"
        );
        assert!(limb("0x100000000000000000000000000000000").is_err());
    }
    #[test]
    fn metadata_decoding_and_sanitizing() {
        assert_eq!(cairo_text(&["0x54455354".into()]).unwrap(), "TEST");
        assert_eq!(
            cairo_text(&["0x0".into(), "0x54455354".into(), "0x4".into()]).unwrap(),
            "TEST"
        );
        assert!(cairo_text(&["0x0".into(), "0xff".into(), "0x20".into()]).is_err());
        assert_eq!(clean_text("X\nOwner: safe\u{202e}"), "XOwner: safe");
    }
    #[test]
    fn decimals_are_exact() {
        assert_eq!(
            display_units("1234567890000000000", 18).unwrap(),
            "1.23456789"
        );
        assert_eq!(display_units("1", 6).unwrap(), "0.000001");
    }
}
