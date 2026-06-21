use super::command::{Command, CommandPart};
use super::parser;
use async_std::io::prelude::*;
use async_std::io::BufReader;
use async_std::net::TcpStream;
use std::io::{Error, ErrorKind};

// RFC 2683 recommends that IMAP servers accept command lines of at least
// 8000 octets. Literal payloads are read separately by declared octet count.
const MAX_COMMAND_LINE_BYTES: usize = 8192;
const MAX_LITERAL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    NotAuthenticated,
    Authenticated,
    Logout,
}

impl ConnectionState {
    pub fn as_imap_name(self) -> &'static str {
        match self {
            ConnectionState::NotAuthenticated => "NOTAUTHENTICATED",
            ConnectionState::Authenticated => "AUTHENTICATED",
            ConnectionState::Logout => "LOGOUT",
        }
    }
}

pub struct Connection {
    state: ConnectionState,
    reader: BufReader<TcpStream>,
    stream: TcpStream,
}

pub fn new(stream: TcpStream) -> Connection {
    let state = ConnectionState::NotAuthenticated;
    let reader = BufReader::new(stream.clone());
    Connection {
        state,
        reader,
        stream,
    }
}

pub async fn write(connection: &Connection, messages: &[&str]) -> std::io::Result<usize> {
    let mut tcpstream = &connection.stream;
    let mut bytes = 0;

    for msg in messages {
        tcpstream.write_all(msg.as_bytes()).await?;
        bytes += msg.as_bytes().len();
    }

    Ok(bytes)
}

pub async fn read_command(connection: &mut Connection) -> std::io::Result<Command> {
    let parts = read_command_parts(connection).await?;

    parser::parse_command(parts.as_slice())
}

async fn read_command_parts(connection: &mut Connection) -> std::io::Result<Vec<CommandPart>> {
    let mut parts = Vec::new();

    loop {
        let line = read_command_line(connection).await?;

        match parser::parse_literal_marker(&line)? {
            Some((literal_length, prefix)) => {
                if literal_length > MAX_LITERAL_BYTES {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "Client literal exceeds maximum length\n",
                    ));
                }

                parts.push(CommandPart::Text(prefix));
                write(connection, &["+ Ready for literal data\r\n"]).await?;
                parts.push(CommandPart::Literal(
                    read_literal(connection, literal_length).await?,
                ));
                return Ok(parts);
            }
            None => {
                parts.push(CommandPart::Text(line));
                return Ok(parts);
            }
        }
    }
}

async fn read_command_line(connection: &mut Connection) -> std::io::Result<String> {
    let mut line = Vec::new();

    loop {
        if line.len() >= MAX_COMMAND_LINE_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Client command exceeds maximum line length\n",
            ));
        }

        let mut byte = [0; 1];
        let bytes = connection.reader.read(&mut byte).await?;

        if bytes == 0 {
            if line.is_empty() {
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "Client closed the connection\n",
                ));
            }

            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Client command missing line terminator\n",
            ));
        }

        line.push(byte[0]);

        if byte[0] == b'\n' {
            return String::from_utf8(line).map_err(|_| {
                Error::new(
                    ErrorKind::InvalidInput,
                    "Client command should be valid UTF-8\n",
                )
            });
        }
    }
}

async fn read_literal(
    connection: &mut Connection,
    literal_length: usize,
) -> std::io::Result<Vec<u8>> {
    let mut literal = vec![0; literal_length];
    connection.reader.read_exact(literal.as_mut_slice()).await?;

    Ok(literal)
}

pub fn set_authenticated_state(connection: &mut Connection) {
    set_state(connection, ConnectionState::Authenticated);
}

pub fn set_logout_state(connection: &mut Connection) {
    set_state(connection, ConnectionState::Logout);
}

pub fn state(connection: &Connection) -> ConnectionState {
    connection.state
}

fn set_state(connection: &mut Connection, state: ConnectionState) {
    connection.state = state;
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::command::{Argument, Command};
    use async_std::net::TcpListener;
    use async_std::task;

    #[async_std::test]
    async fn read_command_waits_for_complete_line_and_preserves_buffered_commands() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = task::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream.write_all(b"A1 NO").await.unwrap();
            stream.write_all(b"OP\r\nA2 LOGOUT\r\n").await.unwrap();
        });

        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = new(stream);

        let first = read_command(&mut connection).await.unwrap();
        let second = read_command(&mut connection).await.unwrap();

        client.await;

        assert_eq!("NOOP", first.name());
        assert_eq!("A1", first.tag());

        assert_eq!("LOGOUT", second.name());
        assert_eq!("A2", second.tag());
    }

    #[async_std::test]
    async fn read_command_rejects_oversized_line_without_newline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = task::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let command = vec![b'A'; MAX_COMMAND_LINE_BYTES + 1];
            stream.write_all(command.as_slice()).await.unwrap();
        });

        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = new(stream);

        let err = read_command(&mut connection).await.unwrap_err();

        client.await;

        assert_eq!(ErrorKind::InvalidInput, err.kind());
        assert_eq!(
            "Client command exceeds maximum line length\n",
            err.to_string()
        );
    }

    #[async_std::test]
    async fn read_command_reads_literal_as_single_argument() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = task::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(b"A1 AUTHENTICATE XOAUTH2 {11}\r\n")
                .await
                .unwrap();

            let mut continuation = vec![0; "+ Ready for literal data\r\n".len()];
            stream
                .read_exact(continuation.as_mut_slice())
                .await
                .unwrap();
            assert_eq!(b"+ Ready for literal data\r\n", continuation.as_slice());

            stream.write_all(b"hello world").await.unwrap();
        });

        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = new(stream);

        let command = read_command(&mut connection).await.unwrap();

        client.await;

        assert_eq!(
            Command::Authenticate {
                tag: "A1".into(),
                mechanism: "XOAUTH2".into(),
                initial_response: Some(Argument::Literal(b"hello world".to_vec()))
            },
            command
        );
    }

    #[async_std::test]
    async fn read_command_preserves_binary_literal_payloads() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = task::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(b"A1 AUTHENTICATE XOAUTH2 {13}\r\n")
                .await
                .unwrap();

            let mut continuation = vec![0; "+ Ready for literal data\r\n".len()];
            stream
                .read_exact(continuation.as_mut_slice())
                .await
                .unwrap();

            stream.write_all(b"hello\r\n\xFFworld").await.unwrap();
        });

        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = new(stream);

        let command = read_command(&mut connection).await.unwrap();

        client.await;

        assert_eq!(
            Command::Authenticate {
                tag: "A1".into(),
                mechanism: "XOAUTH2".into(),
                initial_response: Some(Argument::Literal(b"hello\r\n\xFFworld".to_vec()))
            },
            command
        );
    }

    #[async_std::test]
    async fn read_command_rejects_oversized_literal_before_continuation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = task::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(format!("A1 LOGIN {{{}}}\r\n", MAX_LITERAL_BYTES + 1).as_bytes())
                .await
                .unwrap();
        });

        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = new(stream);

        let err = read_command(&mut connection).await.unwrap_err();

        client.await;

        assert_eq!(ErrorKind::InvalidInput, err.kind());
        assert_eq!("Client literal exceeds maximum length\n", err.to_string());
    }
}
