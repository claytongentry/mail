use async_std::io::prelude::*;
use async_std::io::BufReader;
use async_std::net::TcpStream;
use std::io::{Error, ErrorKind};

// RFC 2683 recommends that IMAP servers accept command lines of at least
// 8000 octets. Literal payloads are read separately by declared octet count.
const MAX_COMMAND_LINE_BYTES: usize = 8192;
const MAX_LITERAL_BYTES: usize = 16 * 1024 * 1024;

enum CommandPart {
    Text(String),
    Literal(String),
}

enum ConnectionState {
    NOTAUTHENTICATED,
    AUTHENTICATED,
    LOGOUT,
}

pub struct Connection {
    state: ConnectionState,
    reader: BufReader<TcpStream>,
    stream: TcpStream,
}

pub fn new(stream: TcpStream) -> Connection {
    let state = ConnectionState::NOTAUTHENTICATED;
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

pub async fn read_command(
    connection: &mut Connection,
) -> std::io::Result<(String, String, Vec<String>)> {
    let parts = read_command_parts(connection).await?;

    parse_command_parts(parts.as_slice())
}

async fn read_command_parts(connection: &mut Connection) -> std::io::Result<Vec<CommandPart>> {
    let mut parts = Vec::new();

    loop {
        let line = read_command_line(connection).await?;

        match parse_literal_marker(&line)? {
            Some((literal_length, prefix)) => {
                parts.push(CommandPart::Text(prefix));
                write(connection, &["+ Ready for literal data\r\n"]).await?;
                parts.push(CommandPart::Literal(
                    read_literal(connection, literal_length).await?,
                ));
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

fn parse_command(line: &str) -> std::io::Result<(String, String, Vec<String>)> {
    parse_command_parts(&[CommandPart::Text(line.to_string())])
}

fn parse_command_parts(parts: &[CommandPart]) -> std::io::Result<(String, String, Vec<String>)> {
    let mut tokens = Vec::new();

    for part in parts {
        match part {
            CommandPart::Text(text) => {
                let text = text.trim_end_matches(&['\r', '\n'][..]);
                tokens.extend(text.split_whitespace().map(String::from));
            }
            CommandPart::Literal(literal) => tokens.push(literal.to_string()),
        }
    }

    if tokens.len() < 2 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Client commands should have at least an identifier and a valid IMAPrev1 command\n",
        ));
    }

    let tag = tokens[0].to_string();
    let command = tokens[1].to_string();
    let args = tokens[2..].to_vec();

    Ok((command, tag, args))
}

fn parse_literal_marker(line: &str) -> std::io::Result<Option<(usize, String)>> {
    let line = line.trim_end_matches(&['\r', '\n'][..]);

    if !line.ends_with('}') {
        return Ok(None);
    }

    let Some(marker_start) = line.rfind('{') else {
        return Ok(None);
    };

    let literal_length = &line[marker_start + 1..line.len() - 1];

    if literal_length.is_empty() || !literal_length.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(None);
    }

    let literal_length = literal_length.parse::<usize>().map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "Client literal length is not valid\n",
        )
    })?;

    if literal_length > MAX_LITERAL_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Client literal exceeds maximum length\n",
        ));
    }

    Ok(Some((literal_length, line[..marker_start].to_string())))
}

async fn read_literal(
    connection: &mut Connection,
    literal_length: usize,
) -> std::io::Result<String> {
    let mut literal = vec![0; literal_length];
    connection.reader.read_exact(literal.as_mut_slice()).await?;

    String::from_utf8(literal).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "Client literal should be valid UTF-8\n",
        )
    })
}

pub fn set_authenticated_state(connection: &mut Connection) {
    set_state(connection, ConnectionState::AUTHENTICATED);
}

pub fn set_logout_state(connection: &mut Connection) {
    set_state(connection, ConnectionState::LOGOUT);
}

fn set_state(connection: &mut Connection, state: ConnectionState) {
    connection.state = state;
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_std::net::TcpListener;
    use async_std::task;

    #[test]
    fn parse_command_trims_crlf() {
        let (command, tag, args) = parse_command("A1 NOOP\r\n").unwrap();

        assert_eq!("NOOP", command);
        assert_eq!("A1", tag);
        assert!(args.is_empty());
    }

    #[test]
    fn parse_command_preserves_argument_tail() {
        let (command, tag, args) =
            parse_command("A2 AUTHENTICATE XOAUTH2 token value\r\n").unwrap();

        assert_eq!("AUTHENTICATE", command);
        assert_eq!("A2", tag);
        assert_eq!(vec!["XOAUTH2", "token", "value"], args);
    }

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

        assert_eq!(("NOOP".to_string(), "A1".to_string(), vec![]), first);
        assert_eq!(("LOGOUT".to_string(), "A2".to_string(), vec![]), second);
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

            stream.write_all(b"hello world\r\n").await.unwrap();
        });

        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = new(stream);

        let (command, tag, args) = read_command(&mut connection).await.unwrap();

        client.await;

        assert_eq!("AUTHENTICATE", command);
        assert_eq!("A1", tag);
        assert_eq!(vec!["XOAUTH2", "hello world"], args);
    }

    #[async_std::test]
    async fn read_command_allows_literal_payloads_to_contain_crlf() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = task::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream.write_all(b"A1 LOGIN {12}\r\n").await.unwrap();

            let mut continuation = vec![0; "+ Ready for literal data\r\n".len()];
            stream
                .read_exact(continuation.as_mut_slice())
                .await
                .unwrap();

            stream.write_all(b"hello\r\nworld\r\n").await.unwrap();
        });

        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = new(stream);

        let (command, tag, args) = read_command(&mut connection).await.unwrap();

        client.await;

        assert_eq!("LOGIN", command);
        assert_eq!("A1", tag);
        assert_eq!(vec!["hello\r\nworld"], args);
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
