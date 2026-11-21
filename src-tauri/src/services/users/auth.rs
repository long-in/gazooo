//! ロックの検証(Argon2id)と総当たり対策(docs/36 §7)。
//!
//! 保存形式: `argon2id$m=<KiB>,t=<回>,p=<並列>$<ソルト hex>$<ハッシュ hex>`。
//! 平文は保存・ログ出力しない。

use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::{AppError, AppResult};

#[cfg(not(test))]
const M_COST: u32 = Params::DEFAULT_M_COST;
#[cfg(not(test))]
const T_COST: u32 = Params::DEFAULT_T_COST;
// テストでは軽いパラメータにして時間を短縮する
#[cfg(test)]
const M_COST: u32 = 64;
#[cfg(test)]
const T_COST: u32 = 1;
const P_COST: u32 = 1;
const OUT_LEN: usize = 32;

/// 連続失敗がこの回数に達すると待機が始まる。
pub const FREE_ATTEMPTS: u32 = 5;
const BASE_LOCKOUT_MS: i64 = 30_000;
const MAX_LOCKOUT_MS: i64 = 15 * 60_000;

pub const PIN_MIN: usize = 4;
pub const PIN_MAX: usize = 8;
pub const PASSPHRASE_MIN: usize = 8;
pub const PASSPHRASE_MAX: usize = 128;

fn argon(m: u32, t: u32, p: u32) -> AppResult<Argon2<'static>> {
    let params = Params::new(m, t, p, Some(OUT_LEN))
        .map_err(|e| AppError::internal(format!("argon2 params: {e}")))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub fn hash_secret(secret: &str) -> AppResult<String> {
    // ソルトは OS の乱数(UUID v4)から 16 バイト
    let salt = *uuid::Uuid::new_v4().as_bytes();
    let mut out = [0u8; OUT_LEN];
    argon(M_COST, T_COST, P_COST)?
        .hash_password_into(secret.as_bytes(), &salt, &mut out)
        .map_err(|e| AppError::internal(format!("argon2: {e}")))?;
    Ok(format!(
        "argon2id$m={M_COST},t={T_COST},p={P_COST}${}${}",
        hex(&salt),
        hex(&out)
    ))
}

pub fn verify_secret(secret: &str, stored: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some("argon2id"), Some(params), Some(salt), Some(hash), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    let nums: Vec<u32> = params
        .split(',')
        .filter_map(|kv| kv.split_once('=').and_then(|(_, v)| v.parse().ok()))
        .collect();
    let [m, t, p] = nums[..] else {
        return false;
    };
    let (Some(salt), Some(expected)) = (unhex(salt), unhex(hash)) else {
        return false;
    };
    let Ok(a) = argon(m, t, p) else {
        return false;
    };
    let mut out = vec![0u8; expected.len()];
    if a.hash_password_into(secret.as_bytes(), &salt, &mut out)
        .is_err()
    {
        return false;
    }
    // 定数時間で比較する
    out.len() == expected.len()
        && out
            .iter()
            .zip(expected.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// 連続失敗 `failed` 回のときの待機時間(ミリ秒)。待機が無いときは `None`。
/// 5 回で 30 秒、以降は失敗のたびに 2 倍(最大 15 分)。
pub fn lockout_ms(failed: u32) -> Option<i64> {
    if failed < FREE_ATTEMPTS {
        return None;
    }
    let shift = (failed - FREE_ATTEMPTS).min(10);
    Some((BASE_LOCKOUT_MS << shift).min(MAX_LOCKOUT_MS))
}

pub fn validate_pin(pin: &str, length: usize) -> AppResult<()> {
    if !(PIN_MIN..=PIN_MAX).contains(&length) {
        return Err(AppError::validation(
            "pinLength",
            "PIN の桁数は 4〜8 で指定してください",
        ));
    }
    if pin.chars().count() != length || !pin.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::validation(
            "secret",
            "PIN は指定した桁数の数字で入力してください",
        ));
    }
    Ok(())
}

pub fn validate_passphrase(p: &str) -> AppResult<()> {
    let n = p.chars().count();
    if !(PASSPHRASE_MIN..=PASSPHRASE_MAX).contains(&n) {
        return Err(AppError::validation(
            "secret",
            "パスフレーズは 8〜128 文字で入力してください",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip_and_salt() {
        let h = hash_secret("correct horse").unwrap();
        assert!(verify_secret("correct horse", &h));
        assert!(!verify_secret("correct horsf", &h));
        assert!(!h.contains("correct"));
        assert_ne!(h, hash_secret("correct horse").unwrap());
        assert!(!verify_secret("x", "garbage"));
    }

    #[test]
    fn lockout_schedule() {
        assert_eq!(lockout_ms(4), None);
        assert_eq!(lockout_ms(5), Some(30_000));
        assert_eq!(lockout_ms(6), Some(60_000));
        assert_eq!(lockout_ms(7), Some(120_000));
        assert_eq!(lockout_ms(100), Some(900_000));
    }

    #[test]
    fn validators() {
        assert!(validate_pin("1234", 4).is_ok());
        assert!(validate_pin("123", 4).is_err());
        assert!(validate_pin("12a4", 4).is_err());
        assert!(validate_pin("1234", 3).is_err());
        assert!(validate_pin("12345678", 8).is_ok());
        assert!(validate_passphrase("short").is_err());
        assert!(validate_passphrase("long enough!").is_ok());
    }
}
