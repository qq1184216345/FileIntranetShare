use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// JWT payload
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    /// 过期时间（秒级时间戳）
    pub exp: i64,
    /// 颁发时间
    pub iat: i64,
}

/// Token 有效期：7 天
const TOKEN_TTL_SECS: i64 = 7 * 24 * 3600;

pub fn hash_password(raw: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(raw.as_bytes(), &salt)
        .map(|p| p.to_string())
        .map_err(|e| format!("hash: {e}"))
}

pub fn verify_password(raw: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(raw.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

pub fn issue_token(secret: &str, subject: &str) -> Result<String, String> {
    let now = super::state::now_secs();
    let claims = Claims {
        sub: subject.to_string(),
        iat: now,
        exp: now + TOKEN_TTL_SECS,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| format!("encode jwt: {e}"))
}

pub fn verify_token(secret: &str, token: &str) -> Result<Claims, String> {
    let mut validation = Validation::default();
    validation.leeway = 10;
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|d| d.claims)
    .map_err(|e| format!("verify jwt: {e}"))
}

/// 随机生成 32 字节十六进制字符串，用作 JWT secret
pub fn random_secret() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}
