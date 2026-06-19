use async_std::io::prelude::*;
use async_std::io::BufReader;
use async_std::net::TcpStream;
use std::io::{Error, ErrorKind};

// RFC 2683 recommends that IMAP servers accept command lines of at least
// 8000 octets. Literal payloads are read separately by declared octet count.
const MAX_COMMAND_LINE_BYTES: usize = 8192;
const MAX_LITERAL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Command {
    pub tag: String,
    pub name: String,
    pub args: Vec<Argument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Argument {
    Atom(String),
    Quoted(String),
    Literal(Vec<u8>),
    List(Vec<Argument>),
    Nil,
}

impl Argument {
    pub fn as_utf8(&self) -> Option<&str> {
        match self {
            Argument::Atom(value) | Argument::Quoted(value) => Some(value),
            Argument::Literal(value) => std::str::from_utf8(value).ok(),
            Argument::List(_) | Argument::Nil => None,
        }
    }
}

enum CommandPart {
    Text(String),
    Literal(Vec<u8>),
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

pub async fn read_command(connection: &mut Connection) -> std::io::Result<Command> {
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

fn parse_command(line: &str) -> std::io::Result<Command> {
    parse_command_parts(&[CommandPart::Text(line.to_string())])
}

fn parse_command_parts(parts: &[CommandPart]) -> std::io::Result<Command> {
    let mut arguments = Vec::new();

    for part in parts {
        match part {
            CommandPart::Text(text) => {
                let text = text.trim_end_matches(&['\r', '\n'][..]);
                arguments.extend(parse_text_arguments(text)?);
            }
            CommandPart::Literal(literal) => arguments.push(Argument::Literal(literal.to_vec())),
        }
    }

    if arguments.len() < 2 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Client commands should have at least an identifier and a valid IMAPrev1 command\n",
        ));
    }

    let tag = command_text(&arguments[0])?;
    let name = command_text(&arguments[1])?;
    let args = arguments[2..].to_vec();

    Ok(Command { tag, name, args })
}

fn parse_text_arguments(text: &str) -> std::io::Result<Vec<Argument>> {
    let mut parser = ArgumentParser::new(text);
    parser.parse_arguments()
}

fn command_text(argument: &Argument) -> std::io::Result<String> {
    match argument {
        Argument::Atom(value) | Argument::Quoted(value) => Ok(value.to_string()),
        _ => Err(Error::new(
            ErrorKind::InvalidInput,
            "Client command identifier and name should be text arguments\n",
        )),
    }
}

struct ArgumentParser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> ArgumentParser<'a> {
    fn new(input: &'a str) -> ArgumentParser<'a> {
        ArgumentParser {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn parse_arguments(&mut self) -> std::io::Result<Vec<Argument>> {
        let mut arguments = Vec::new();

        loop {
            self.skip_whitespace();

            if self.is_at_end() {
                return Ok(arguments);
            }

            arguments.push(self.parse_argument()?);
        }
    }

    fn parse_argument(&mut self) -> std::io::Result<Argument> {
        match self.peek() {
            Some(b'"') => self.parse_quoted(),
            Some(b'(') => self.parse_list(),
            Some(b')') => Err(Error::new(
                ErrorKind::InvalidInput,
                "Client command contains an unexpected list terminator\n",
            )),
            Some(_) => self.parse_atom(),
            None => Err(Error::new(
                ErrorKind::InvalidInput,
                "Client command contains an empty argument\n",
            )),
        }
    }

    fn parse_list(&mut self) -> std::io::Result<Argument> {
        self.position += 1;
        let mut values = Vec::new();

        loop {
            self.skip_whitespace();

            match self.peek() {
                Some(b')') => {
                    self.position += 1;
                    return Ok(Argument::List(values));
                }
                Some(_) => values.push(self.parse_argument()?),
                None => {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "Client command contains an unterminated list\n",
                    ))
                }
            }
        }
    }

    fn parse_quoted(&mut self) -> std::io::Result<Argument> {
        self.position += 1;
        let mut value = Vec::new();

        while let Some(byte) = self.next() {
            match byte {
                b'"' => {
                    return String::from_utf8(value).map(Argument::Quoted).map_err(|_| {
                        Error::new(
                            ErrorKind::InvalidInput,
                            "Client quoted argument should be valid UTF-8\n",
                        )
                    })
                }
                b'\\' => match self.next() {
                    Some(escaped) => value.push(escaped),
                    None => {
                        return Err(Error::new(
                            ErrorKind::InvalidInput,
                            "Client quoted argument contains an incomplete escape\n",
                        ))
                    }
                },
                other => value.push(other),
            }
        }

        Err(Error::new(
            ErrorKind::InvalidInput,
            "Client command contains an unterminated quoted argument\n",
        ))
    }

    fn parse_atom(&mut self) -> std::io::Result<Argument> {
        let start = self.position;

        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() || byte == b'(' || byte == b')' {
                break;
            }

            self.position += 1;
        }

        if start == self.position {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Client command contains an empty atom\n",
            ));
        }

        let atom = std::str::from_utf8(&self.input[start..self.position]).map_err(|_| {
            Error::new(
                ErrorKind::InvalidInput,
                "Client atom argument should be valid UTF-8\n",
            )
        })?;

        if atom.eq_ignore_ascii_case("NIL") {
            Ok(Argument::Nil)
        } else {
            Ok(Argument::Atom(atom.to_string()))
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(byte) if byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.input.len()
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }
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
) -> std::io::Result<Vec<u8>> {
    let mut literal = vec![0; literal_length];
    connection.reader.read_exact(literal.as_mut_slice()).await?;

    Ok(literal)
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
        let command = parse_command("A1 NOOP\r\n").unwrap();

        assert_eq!("NOOP", command.name);
        assert_eq!("A1", command.tag);
        assert!(command.args.is_empty());
    }

    #[test]
    fn parse_command_preserves_argument_tail() {
        let command = parse_command("A2 AUTHENTICATE XOAUTH2 token value\r\n").unwrap();

        assert_eq!("AUTHENTICATE", command.name);
        assert_eq!("A2", command.tag);
        assert_eq!(
            vec![
                Argument::Atom("XOAUTH2".to_string()),
                Argument::Atom("token".to_string()),
                Argument::Atom("value".to_string())
            ],
            command.args
        );
    }

    #[test]
    fn parse_command_preserves_typed_arguments() {
        let command = parse_command(r#"A3 STORE 1 (\Seen "two words" NIL)"#).unwrap();

        assert_eq!("STORE", command.name);
        assert_eq!("A3", command.tag);
        assert_eq!(
            vec![
                Argument::Atom("1".to_string()),
                Argument::List(vec![
                    Argument::Atom("\\Seen".to_string()),
                    Argument::Quoted("two words".to_string()),
                    Argument::Nil,
                ])
            ],
            command.args
        );
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

        assert_eq!("NOOP", first.name);
        assert_eq!("A1", first.tag);
        assert!(first.args.is_empty());

        assert_eq!("LOGOUT", second.name);
        assert_eq!("A2", second.tag);
        assert!(second.args.is_empty());
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

        assert_eq!("AUTHENTICATE", command.name);
        assert_eq!("A1", command.tag);
        assert_eq!(
            vec![
                Argument::Atom("XOAUTH2".to_string()),
                Argument::Literal(b"hello world".to_vec())
            ],
            command.args
        );
    }

    #[async_std::test]
    async fn read_command_preserves_binary_literal_payloads() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = task::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream.write_all(b"A1 LOGIN {13}\r\n").await.unwrap();

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

        assert_eq!("LOGIN", command.name);
        assert_eq!("A1", command.tag);
        assert_eq!(
            vec![Argument::Literal(b"hello\r\n\xFFworld".to_vec())],
            command.args
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
