use async_std::io::prelude::*;
use async_std::io::BufReader;
use async_std::net::TcpStream;
use std::io::{Error, ErrorKind};

// RFC 2683 recommends that IMAP servers accept command lines of at least
// 8000 octets. This cap is for the command line only; IMAP literals should
// be handled separately by reading their declared octet count after a
// continuation response.
const MAX_COMMAND_LINE_BYTES: usize = 8192;

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
    let line = read_command_line(connection).await?;

    parse_command(line.as_str())
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
    let command = line.trim_end_matches(&['\r', '\n'][..]);

    let v: Vec<&str> = command.splitn(3, ' ').collect();

    let length = v.len();
    if length > 3 || length < 2 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Client commands should have at least an identifier and a valid IMAPrev1 command\n",
        ));
    }

    let command = v[1].to_string();
    let tag = v[0].to_string();
    let mut args: Vec<String> = Vec::new();

    if length == 3 {
        args = v[2].split(' ').map(String::from).collect();
    }

    Ok((command, tag, args))
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
}
