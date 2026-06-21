use crate::imap::command::Argument;
use base64::{engine::general_purpose, Engine as _};
use std::io::{Error, ErrorKind};

pub fn bearer_token(initial_response: &Option<Argument>) -> std::io::Result<String> {
    let encoded = initial_response
        .as_ref()
        .and_then(Argument::as_utf8)
        .ok_or_else(invalid_initial_response)?;

    if encoded == "=" {
        return Err(invalid_initial_response());
    }

    let decoded = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| invalid_initial_response())?;
    let decoded = std::str::from_utf8(&decoded).map_err(|_| invalid_initial_response())?;

    if !decoded.ends_with("\x01\x01") {
        return Err(invalid_initial_response());
    }

    let mut user = None;
    let mut bearer_token = None;

    for field in decoded.split('\x01') {
        if field.is_empty() {
            continue;
        }

        if let Some(value) = field.strip_prefix("user=") {
            if !value.is_empty() {
                user = Some(value);
            }
            continue;
        }

        if let Some(value) = field.strip_prefix("auth=") {
            let Some(token) = value.strip_prefix("Bearer ") else {
                return Err(invalid_initial_response());
            };

            if !token.is_empty() {
                bearer_token = Some(token);
            }
        }
    }

    match (user, bearer_token) {
        (Some(_user), Some(token)) => Ok(token.to_string()),
        _ => Err(invalid_initial_response()),
    }
}

fn invalid_initial_response() -> Error {
    Error::new(ErrorKind::InvalidInput, "Invalid XOAUTH2 initial response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;

    fn initial_response(value: &str) -> Option<Argument> {
        Some(Argument::Atom(value.to_string()))
    }

    fn encode(value: &str) -> Option<Argument> {
        initial_response(&general_purpose::STANDARD.encode(value))
    }

    #[test]
    fn extracts_bearer_token_from_valid_initial_response() {
        let token = bearer_token(&encode(
            "user=test@example.com\x01auth=Bearer token-value\x01\x01",
        ))
        .unwrap();

        assert_eq!("token-value", token);
    }

    #[test]
    fn rejects_raw_token_initial_response() {
        let err = bearer_token(&initial_response("token-value")).unwrap_err();

        assert_eq!(ErrorKind::InvalidInput, err.kind());
        assert_eq!("Invalid XOAUTH2 initial response", err.to_string());
    }

    #[test]
    fn rejects_missing_terminator() {
        let err =
            bearer_token(&encode("user=test@example.com\x01auth=Bearer token-value")).unwrap_err();

        assert_eq!(ErrorKind::InvalidInput, err.kind());
    }
}
