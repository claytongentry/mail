use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, TokenData, Validation,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    exp: u64,
}

fn get_key() -> std::string::String {
    env::var("JWT_SECRET").unwrap()
}

pub fn authenticate(
    token: &String,
) -> std::result::Result<TokenData<Claims>, jsonwebtoken::errors::Error> {
    let key = get_key();
    let key_ref = key.as_ref();

    decode::<Claims>(
        &token,
        &DecodingKey::from_secret(key_ref),
        &Validation::new(Algorithm::HS256),
    )
}

fn issue() -> String {
    let one_hour = Duration::new(60 * 60, 0);
    let in_one_hour = SystemTime::now() + one_hour;
    let exp = in_one_hour.duration_since(UNIX_EPOCH).unwrap().as_secs();

    let claims = Claims { exp: exp };

    let key = get_key();
    let key_ref = key.as_ref();

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(key_ref),
    )
    .unwrap()
}
