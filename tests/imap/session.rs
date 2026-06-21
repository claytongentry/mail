use async_std::io::prelude::*;
use async_std::io::BufReader;
use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use base64::{engine::general_purpose, Engine as _};
use jsonwebtoken::{encode, EncodingKey, Header};
use mail::imap::{connection, session};
use mail::store::{
    FixtureMailStore, MailStore, MailStoreResult, MailboxSelection, MessageFlag, SqliteMailStore,
};
use serde::Serialize;
use std::env;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
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
        let store = FixtureMailStore;
        session::handle_connection(&mut connection, &store).await;
    });

    let client = TcpStream::connect(addr).await.unwrap();

    (BufReader::new(client), server)
}

async fn connect_to_server_with_store(
    store: impl MailStore + Send + 'static,
) -> (BufReader<TcpStream>, task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = task::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = connection::new(stream);
        session::handle_connection(&mut connection, &store).await;
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

async fn authenticate_client(reader: &mut BufReader<TcpStream>, secret: &str) {
    let token = test_token(secret);
    let xoauth2 = xoauth2_initial_response(&token);

    write_line(
        reader,
        &format!("A1 AUTHENTICATE XOAUTH2 {}\r\n", xoauth2),
    )
    .await;
    assert_eq!(
        "A1 OK SASL authentication successful\r\n",
        read_line(reader).await
    );
}

async fn assert_fixture_select_response(reader: &mut BufReader<TcpStream>, tag: &str) {
    assert_eq!("* 172 EXISTS\r\n", read_line(reader).await);
    assert_eq!("* 1 RECENT\r\n", read_line(reader).await);
    assert_eq!(
        "* OK [UNSEEN 12] Message 12 is first unseen\r\n",
        read_line(reader).await
    );
    assert_eq!(
        "* OK [UIDVALIDITY 3857529045] UIDs valid\r\n",
        read_line(reader).await
    );
    assert_eq!(
        "* OK [UIDNEXT 4392] Predicted next UID\r\n",
        read_line(reader).await
    );
    assert_eq!(
        "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
        read_line(reader).await
    );
    assert_eq!(
        "* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n",
        read_line(reader).await
    );
    assert_eq!(
        format!("{} OK [READ-WRITE] SELECT completed\r\n", tag),
        read_line(reader).await
    );
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

fn unique_sqlite_path() -> String {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir()
        .join(format!("mail-store-{}.sqlite3", id))
        .to_string_lossy()
        .to_string()
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
async fn capability_advertises_xoauth2_sasl_ir_and_login_disabled() {
    let (mut reader, server) = connect_to_server().await;

    read_line(&mut reader).await;
    write_line(&mut reader, "A1 CAPABILITY\r\n").await;

    assert_eq!(
        "* CAPABILITY IMAP4rev1 AUTH=XOAUTH2 LOGINDISABLED SASL-IR\r\n",
        read_line(&mut reader).await
    );
    assert_eq!("A1 OK CAPABILITY completed\r\n", read_line(&mut reader).await);

    logout(&mut reader, server).await;
}

#[async_std::test]
async fn authenticate_success_is_tagged_and_crlf_terminated() {
    let _guard = lock_env();
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
    let _guard = lock_env();
    let secret = "test-secret";
    unsafe {
        env::set_var("JWT_SECRET", secret);
    }
    let (mut reader, server) = connect_to_server().await;

    read_line(&mut reader).await;
    authenticate_client(&mut reader, secret).await;

    write_line(&mut reader, "A2 SELECT INBOX\r\n").await;
    assert_fixture_select_response(&mut reader, "A2").await;

    logout(&mut reader, server).await;
}

#[async_std::test]
async fn select_inbox_is_case_insensitive() {
    let _guard = lock_env();
    let secret = "test-secret";
    unsafe {
        env::set_var("JWT_SECRET", secret);
    }
    let (mut reader, server) = connect_to_server().await;

    read_line(&mut reader).await;
    authenticate_client(&mut reader, secret).await;

    write_line(&mut reader, "A2 SELECT inbox\r\n").await;
    assert_fixture_select_response(&mut reader, "A2").await;

    logout(&mut reader, server).await;
}

#[async_std::test]
async fn select_response_can_use_seeded_sqlite_store() {
    let _guard = lock_env();
    let secret = "test-secret";
    unsafe {
        env::set_var("JWT_SECRET", secret);
    }
    let path = unique_sqlite_path();
    let store = SqliteMailStore::open(&path).unwrap();
    let (mut reader, server) = connect_to_server_with_store(store).await;

    read_line(&mut reader).await;
    authenticate_client(&mut reader, secret).await;

    write_line(&mut reader, "A2 SELECT INBOX\r\n").await;
    assert_fixture_select_response(&mut reader, "A2").await;

    logout(&mut reader, server).await;
    let _ = std::fs::remove_file(path);
}

#[async_std::test]
async fn missing_literal_mailbox_does_not_inject_response_lines() {
    let _guard = lock_env();
    let secret = "test-secret";
    unsafe {
        env::set_var("JWT_SECRET", secret);
    }
    let (mut reader, server) = connect_to_server().await;

    read_line(&mut reader).await;
    authenticate_client(&mut reader, secret).await;

    write_line(&mut reader, "A2 SELECT {16}\r\n").await;
    assert_eq!("+ Ready for literal data\r\n", read_line(&mut reader).await);
    reader
        .get_mut()
        .write_all(b"x\r\n* OK injected")
        .await
        .unwrap();

    assert_eq!("A2 NO Mailbox does not exist\r\n", read_line(&mut reader).await);

    write_line(&mut reader, "A3 NOOP\r\n").await;
    assert_eq!("A3 OK NOOP completed\r\n", read_line(&mut reader).await);

    logout(&mut reader, server).await;
}

struct TestMailStore {
    selection: MailboxSelection,
}

impl MailStore for TestMailStore {
    fn select_mailbox(&self, _mailbox: &str) -> MailStoreResult<MailboxSelection> {
        Ok(self.selection.clone())
    }
}

#[async_std::test]
async fn select_response_uses_mail_store_selection() {
    let _guard = lock_env();
    let secret = "test-secret";
    unsafe {
        env::set_var("JWT_SECRET", secret);
    }
    let selection = MailboxSelection {
        exists: 3,
        recent: 2,
        first_unseen: Some(7),
        uid_validity: 99,
        uid_next: 123,
        flags: vec![MessageFlag::Seen, MessageFlag::Custom("$Forwarded".to_string())],
        permanent_flags: vec![MessageFlag::Seen],
    };
    let store = TestMailStore { selection };
    let (mut reader, server) = connect_to_server_with_store(store).await;

    read_line(&mut reader).await;
    authenticate_client(&mut reader, secret).await;

    write_line(&mut reader, "A2 SELECT INBOX\r\n").await;
    assert_eq!("* 3 EXISTS\r\n", read_line(&mut reader).await);
    assert_eq!("* 2 RECENT\r\n", read_line(&mut reader).await);
    assert_eq!(
        "* OK [UNSEEN 7] Message 7 is first unseen\r\n",
        read_line(&mut reader).await
    );
    assert_eq!(
        "* OK [UIDVALIDITY 99] UIDs valid\r\n",
        read_line(&mut reader).await
    );
    assert_eq!(
        "* OK [UIDNEXT 123] Predicted next UID\r\n",
        read_line(&mut reader).await
    );
    assert_eq!(
        "* FLAGS (\\Seen $Forwarded)\r\n",
        read_line(&mut reader).await
    );
    assert_eq!(
        "* OK [PERMANENTFLAGS (\\Seen)] Limited\r\n",
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
    let _guard = lock_env();
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
    let _guard = lock_env();
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
    let _guard = lock_env();
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
