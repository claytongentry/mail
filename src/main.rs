use async_std::net::TcpListener;
use base64::{engine::general_purpose, Engine as _};
use futures::stream::StreamExt;
use std::env;
use std::io::{Error, ErrorKind};
mod connection;
mod oauth2;
mod parser;
use connection::{Connection, ConnectionState};
use parser::{Argument, Command};

const GREETING: &str = "* OK IMAP4rev1 Service Ready\r\n";

fn tagged(tag: &str, status: &str, message: &str) -> String {
    format!(
        "{} {} {}\r\n",
        tag,
        status,
        message.trim_end_matches(&['\r', '\n'][..])
    )
}

fn untagged(message: &str) -> String {
    format!("* {}\r\n", message.trim_end_matches(&['\r', '\n'][..]))
}

async fn write_messages(connection: &Connection, messages: Vec<String>) -> std::io::Result<usize> {
    let refs = messages.iter().map(String::as_str).collect::<Vec<_>>();
    connection::write(connection, refs.as_slice()).await
}

async fn ok(connection: &Connection, tag: &str, message: &str) -> std::io::Result<usize> {
    write_messages(connection, vec![tagged(tag, "OK", message)]).await
}

async fn no(connection: &Connection, tag: &str, message: &str) -> std::io::Result<usize> {
    write_messages(connection, vec![tagged(tag, "NO", message)]).await
}

async fn bad(connection: &Connection, message: &str, tag: &str) -> std::io::Result<usize> {
    write_messages(connection, vec![tagged(tag, "BAD", message)]).await
}

/**
 ****************************************************************
 * Begin response implementations
 ****************************************************************
 */

/**
 * https://tools.ietf.org/html/rfc2060#section-6.2.1
 */
async fn authenticate(
    connection: &mut Connection,
    id: &str,
    mechanism: &str,
    initial_response: &Option<Argument>,
) -> std::io::Result<usize> {
    if mechanism.eq_ignore_ascii_case("XOAUTH2") {
        let token = match xoauth2_bearer_token(initial_response) {
            Ok(token) => token,
            Err(err) => return bad(connection, &err.to_string(), id).await,
        };

        match oauth2::authenticate(&token) {
            Ok(_claims) => {
                connection::set_authenticated_state(connection);
                ok(connection, id, "SASL authentication successful").await
            }
            _err => no(connection, id, "Invalid credentials").await,
        }
    } else {
        no(connection, id, "Unsupported authentication mechanism").await
    }
}

fn xoauth2_bearer_token(initial_response: &Option<Argument>) -> std::io::Result<String> {
    let encoded = initial_response
        .as_ref()
        .and_then(Argument::as_utf8)
        .ok_or_else(invalid_xoauth2_response)?;

    if encoded == "=" {
        return Err(invalid_xoauth2_response());
    }

    let decoded = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| invalid_xoauth2_response())?;
    let decoded = std::str::from_utf8(&decoded).map_err(|_| invalid_xoauth2_response())?;

    if !decoded.ends_with("\x01\x01") {
        return Err(invalid_xoauth2_response());
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
                return Err(invalid_xoauth2_response());
            };

            if !token.is_empty() {
                bearer_token = Some(token);
            }
        }
    }

    match (user, bearer_token) {
        (Some(_user), Some(token)) => Ok(token.to_string()),
        _ => Err(invalid_xoauth2_response()),
    }
}

fn invalid_xoauth2_response() -> Error {
    Error::new(ErrorKind::InvalidInput, "Invalid XOAUTH2 initial response")
}

async fn capability(connection: &Connection, id: &str) -> std::io::Result<usize> {
    write_messages(
        connection,
        vec![
            untagged("CAPABILITY IMAP4rev1 AUTH=XOAUTH2 LOGINDISABLED"),
            tagged(id, "OK", "CAPABILITY completed"),
        ],
    )
    .await
}

async fn login(connection: &Connection, id: &str) -> std::io::Result<usize> {
    no(connection, id, "Login is disabled.").await
}

async fn logout(connection: &mut Connection, id: &str) -> std::io::Result<usize> {
    connection::set_logout_state(connection);
    write_messages(
        connection,
        vec![
            untagged("BYE IMAPrev1 Server logging out"),
            tagged(id, "OK", "LOGOUT completed"),
        ],
    )
    .await
}

/**
* TODO:
* Since any command can return a status update as untagged data, the
    NOOP command can be used as a periodic poll for new messages or
    message status updates during a period of inactivity.  The NOOP
    command can also be used to reset any inactivity autologout timer
    on the server.
* https://tools.ietf.org/html/rfc2060#section-6.1.2
*/
async fn noop(connection: &Connection, id: &str) -> std::io::Result<usize> {
    ok(connection, id, "NOOP completed").await
}

async fn select(connection: &Connection, id: &str) -> std::io::Result<usize> {
    write_messages(
        connection,
        vec![
            untagged("172 EXISTS"),
            untagged("1 RECENT"),
            untagged("OK [UNSEEN 12] Message 12 is first unseen"),
            untagged("OK [UIDVALIDITY 3857529045] UIDs valid"),
            untagged("OK [UIDNEXT 4392] Predicted next UID"),
            untagged("FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)"),
            untagged("OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited"),
            tagged(id, "OK", "[READ-WRITE] SELECT completed"),
        ],
    )
    .await
}

/**
 ****************************************************************
 * End response implementations
 ****************************************************************
 */

async fn handle_command(command: &Command, connection: &mut Connection) -> std::io::Result<usize> {
    let state = connection::state(connection);

    if !command_is_valid_for_state(command, state) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Command {} is not valid in {} state",
                command.name(),
                state.as_imap_name()
            ),
        ));
    }

    match command {
        Command::Authenticate {
            tag,
            mechanism,
            initial_response,
        } => authenticate(connection, tag, mechanism, initial_response).await,
        Command::Capability { tag } => capability(connection, tag).await,
        Command::Login { tag, .. } => login(connection, tag).await,
        Command::Logout { tag } => logout(connection, tag).await,
        Command::Noop { tag } => noop(connection, tag).await,
        Command::Select { tag, .. } => select(connection, tag).await,
        Command::Unknown { name, .. } => {
            let message = name.to_string() + " is not a valid command.";
            Err(Error::new(ErrorKind::InvalidInput, message))
        }
    }
}

fn command_is_valid_for_state(command: &Command, state: ConnectionState) -> bool {
    match state {
        ConnectionState::NotAuthenticated => matches!(
            command,
            Command::Authenticate { .. }
                | Command::Capability { .. }
                | Command::Login { .. }
                | Command::Logout { .. }
                | Command::Noop { .. }
                | Command::Unknown { .. }
        ),
        ConnectionState::Authenticated => matches!(
            command,
            Command::Capability { .. }
                | Command::Logout { .. }
                | Command::Noop { .. }
                | Command::Select { .. }
                | Command::Unknown { .. }
        ),
        ConnectionState::Logout => false,
    }
}

async fn handle_connection(connection: &mut Connection) {
    if connection::write(connection, &[GREETING]).await.is_err() {
        return;
    }

    loop {
        match connection::read_command(connection).await {
            Ok(command) => {
                let tag = command.tag().to_string();
                let name = command.name().to_string();
                let _ = match handle_command(&command, connection).await {
                    Ok(val) => Ok(val),
                    Err(err) => match err.kind() {
                        ErrorKind::BrokenPipe => break,
                        ErrorKind::InvalidInput => bad(&connection, &err.to_string(), &tag).await,
                        _other => {
                            eprintln!("Connection error: {}", err);
                            break;
                        }
                    },
                };

                // Can use this space to close connections as needed
                match name.as_str() {
                    "LOGOUT" => break,
                    other => other,
                };
            }
            Err(err) => {
                if err.kind() == ErrorKind::BrokenPipe {
                    break;
                }

                let tag = "*".to_string();
                let msg = &err.to_string();
                let _ = bad(&connection, msg, &tag).await;
            }
        }
    }
}

#[async_std::main]
async fn main() -> std::io::Result<()> {
    let bind_addr = env::var("IMAP_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:1143".to_string());
    println!("IMAPrev1 listening on {}...", bind_addr);

    /*
     * IMAPrev1 servers listen on port 143
     * https://tools.ietf.org/html/rfc2060#section-2.1
     */
    let listener = TcpListener::bind(bind_addr.as_str()).await?;

    // accept connections concurrently
    listener
        .incoming()
        .for_each_concurrent(/* limit */ None, |stream| async move {
            match stream {
                Ok(stream) => {
                    let mut conn = connection::new(stream);
                    handle_connection(&mut conn).await;
                }
                Err(err) => eprintln!("Failed to accept connection: {}", err),
            }
        })
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_std::io::prelude::*;
    use async_std::io::BufReader;
    use async_std::net::{TcpListener, TcpStream};
    use async_std::task;
    use base64::engine::general_purpose;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    static AUTH_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_auth_env() -> std::sync::MutexGuard<'static, ()> {
        AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[derive(Serialize)]
    struct TestClaims {
        exp: u64,
    }

    async fn connect_to_server() -> (BufReader<TcpStream>, task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = task::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = connection::new(stream);
            handle_connection(&mut connection).await;
        });

        let client = TcpStream::connect(addr).await.unwrap();

        (BufReader::new(client), server)
    }

    async fn read_line(reader: &mut BufReader<TcpStream>) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        line
    }

    async fn write_line(reader: &mut BufReader<TcpStream>, line: &str) {
        reader.get_mut().write_all(line.as_bytes()).await.unwrap();
    }

    async fn logout(reader: &mut BufReader<TcpStream>, server: task::JoinHandle<()>) {
        write_line(reader, "ZZ LOGOUT\r\n").await;
        assert_eq!(
            "* BYE IMAPrev1 Server logging out\r\n",
            read_line(reader).await
        );
        assert_eq!("ZZ OK LOGOUT completed\r\n", read_line(reader).await);
        server.await;
    }

    fn test_token(secret: &str) -> String {
        let exp = (SystemTime::now() + Duration::new(60 * 60, 0))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        encode(
            &Header::default(),
            &TestClaims { exp },
            &EncodingKey::from_secret(secret.as_ref()),
        )
        .unwrap()
    }

    fn xoauth2_initial_response(token: &str) -> String {
        general_purpose::STANDARD.encode(format!(
            "user=test@example.com\x01auth=Bearer {}\x01\x01",
            token
        ))
    }

    #[async_std::test]
    async fn connection_starts_with_imap_greeting() {
        let (mut reader, server) = connect_to_server().await;

        assert_eq!(
            "* OK IMAP4rev1 Service Ready\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn select_before_authentication_is_rejected() {
        let (mut reader, server) = connect_to_server().await;

        assert_eq!(
            "* OK IMAP4rev1 Service Ready\r\n",
            read_line(&mut reader).await
        );
        write_line(&mut reader, "A1 SELECT INBOX\r\n").await;

        assert_eq!(
            "A1 BAD Command SELECT is not valid in NOTAUTHENTICATED state\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_success_is_tagged_and_crlf_terminated() {
        let _guard = lock_auth_env();
        let secret = "test-secret";
        unsafe {
            env::set_var("JWT_SECRET", secret);
        }
        let token = test_token(secret);
        let xoauth2 = xoauth2_initial_response(&token);
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", xoauth2),
        )
        .await;

        assert_eq!(
            "A1 OK SASL authentication successful\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn select_response_uses_crlf_and_tagged_completion() {
        let _guard = lock_auth_env();
        let secret = "test-secret";
        unsafe {
            env::set_var("JWT_SECRET", secret);
        }
        let token = test_token(secret);
        let xoauth2 = xoauth2_initial_response(&token);
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", xoauth2),
        )
        .await;
        assert_eq!(
            "A1 OK SASL authentication successful\r\n",
            read_line(&mut reader).await
        );

        write_line(&mut reader, "A2 SELECT INBOX\r\n").await;
        assert_eq!("* 172 EXISTS\r\n", read_line(&mut reader).await);
        assert_eq!("* 1 RECENT\r\n", read_line(&mut reader).await);
        assert_eq!(
            "* OK [UNSEEN 12] Message 12 is first unseen\r\n",
            read_line(&mut reader).await
        );
        assert_eq!(
            "* OK [UIDVALIDITY 3857529045] UIDs valid\r\n",
            read_line(&mut reader).await
        );
        assert_eq!(
            "* OK [UIDNEXT 4392] Predicted next UID\r\n",
            read_line(&mut reader).await
        );
        assert_eq!(
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
            read_line(&mut reader).await
        );
        assert_eq!(
            "* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n",
            read_line(&mut reader).await
        );
        assert_eq!(
            "A2 OK [READ-WRITE] SELECT completed\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_without_jwt_secret_returns_tagged_no() {
        let _guard = lock_auth_env();
        unsafe {
            env::remove_var("JWT_SECRET");
        }
        let xoauth2 = xoauth2_initial_response("token");
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", xoauth2),
        )
        .await;

        assert_eq!(
            "A1 NO Invalid credentials\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_rejects_raw_token_initial_response() {
        let _guard = lock_auth_env();
        let secret = "test-secret";
        unsafe {
            env::set_var("JWT_SECRET", secret);
        }
        let token = test_token(secret);
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", token),
        )
        .await;

        assert_eq!(
            "A1 BAD Invalid XOAUTH2 initial response\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_rejects_malformed_xoauth2_base64() {
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(&mut reader, "A1 AUTHENTICATE XOAUTH2 not-base64!\r\n").await;

        assert_eq!(
            "A1 BAD Invalid XOAUTH2 initial response\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_rejects_xoauth2_without_bearer_token() {
        let initial_response =
            general_purpose::STANDARD.encode("user=test@example.com\x01auth=Bearer \x01\x01");
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", initial_response),
        )
        .await;

        assert_eq!(
            "A1 BAD Invalid XOAUTH2 initial response\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_rejects_xoauth2_unsupported_auth_scheme() {
        let initial_response =
            general_purpose::STANDARD.encode("user=test@example.com\x01auth=Basic token\x01\x01");
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", initial_response),
        )
        .await;

        assert_eq!(
            "A1 BAD Invalid XOAUTH2 initial response\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_rejects_xoauth2_without_terminator() {
        let initial_response =
            general_purpose::STANDARD.encode("user=test@example.com\x01auth=Bearer token");
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", initial_response),
        )
        .await;

        assert_eq!(
            "A1 BAD Invalid XOAUTH2 initial response\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn authenticate_rejects_xoauth2_invalid_bearer_token() {
        let _guard = lock_auth_env();
        unsafe {
            env::set_var("JWT_SECRET", "test-secret");
        }
        let xoauth2 = xoauth2_initial_response("not-a-jwt");
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(
            &mut reader,
            &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", xoauth2),
        )
        .await;

        assert_eq!(
            "A1 NO Invalid credentials\r\n",
            read_line(&mut reader).await
        );

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn malformed_command_returns_bad_and_connection_continues() {
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(&mut reader, "A1 SELECT\r\n").await;

        assert_eq!(
            "* BAD Client command has invalid arguments\r\n",
            read_line(&mut reader).await
        );

        write_line(&mut reader, "A2 NOOP\r\n").await;
        assert_eq!("A2 OK NOOP completed\r\n", read_line(&mut reader).await);

        logout(&mut reader, server).await;
    }

    #[async_std::test]
    async fn client_disconnect_after_greeting_exits_without_panic() {
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        drop(reader);

        server.await;
    }

    #[async_std::test]
    async fn logout_sends_bye_tagged_ok_and_closes_connection() {
        let (mut reader, server) = connect_to_server().await;

        read_line(&mut reader).await;
        write_line(&mut reader, "A1 LOGOUT\r\n").await;

        assert_eq!(
            "* BYE IMAPrev1 Server logging out\r\n",
            read_line(&mut reader).await
        );
        assert_eq!("A1 OK LOGOUT completed\r\n", read_line(&mut reader).await);
        server.await;

        let mut eof = String::new();
        assert_eq!(0, reader.read_line(&mut eof).await.unwrap());
    }
}
