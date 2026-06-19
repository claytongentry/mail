use async_std::io::prelude::*;
use async_std::io::BufReader;
use async_std::net::TcpStream;
use std::io::{Error, ErrorKind};

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
    let mut line = String::new();
    let bytes = connection.reader.read_line(&mut line).await?;

    if bytes == 0 {
        return Err(Error::new(
            ErrorKind::BrokenPipe,
            "Client closed the connection\n",
        ));
    }

    parse_command(&line)
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
}
