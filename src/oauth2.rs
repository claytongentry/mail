use jsonwebtoken::{decode, Algorithm, DecodingKey, TokenData, Validation};
use serde::Deserialize;
use std::env;
use std::fmt;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Claims {
    pub exp: u64,
}

#[derive(Debug)]
pub enum AuthError {
    MissingSecret,
    InvalidToken(jsonwebtoken::errors::Error),
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::MissingSecret => write!(formatter, "JWT_SECRET is not configured"),
            AuthError::InvalidToken(err) => write!(formatter, "{}", err),
        }
    }
}

impl std::error::Error for AuthError {}

fn get_key() -> Result<String, AuthError> {
    env::var("JWT_SECRET").map_err(|_| AuthError::MissingSecret)
}

pub fn authenticate(token: &str) -> Result<TokenData<Claims>, AuthError> {
    let key = get_key()?;
    let key_ref = key.as_ref();

    decode::<Claims>(
        token,
        &DecodingKey::from_secret(key_ref),
        &Validation::new(Algorithm::HS256),
    )
    .map_err(AuthError::InvalidToken)
}
