use super::command::{Argument, Command, CommandPart};
use std::convert::TryInto;
use std::io::{Error, ErrorKind};

pub fn parse_command(parts: &[CommandPart]) -> std::io::Result<Command> {
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

    parse_specific_command(tag, name, args)
}

pub fn parse_literal_marker(line: &str) -> std::io::Result<Option<(usize, String)>> {
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

    Ok(Some((literal_length, line[..marker_start].to_string())))
}

fn parse_specific_command(
    tag: String,
    name: String,
    args: Vec<Argument>,
) -> std::io::Result<Command> {
    if name.eq_ignore_ascii_case("AUTHENTICATE") {
        parse_authenticate(tag, args)
    } else if name.eq_ignore_ascii_case("CAPABILITY") {
        parse_no_arg(tag, args, |tag| Command::Capability { tag })
    } else if name.eq_ignore_ascii_case("LOGIN") {
        parse_login(tag, args)
    } else if name.eq_ignore_ascii_case("LOGOUT") {
        parse_no_arg(tag, args, |tag| Command::Logout { tag })
    } else if name.eq_ignore_ascii_case("NOOP") {
        parse_no_arg(tag, args, |tag| Command::Noop { tag })
    } else if name.eq_ignore_ascii_case("SELECT") {
        parse_select(tag, args)
    } else {
        Ok(Command::Unknown { tag, name, args })
    }
}

fn parse_authenticate(tag: String, args: Vec<Argument>) -> std::io::Result<Command> {
    if args.is_empty() || args.len() > 2 {
        return invalid_arguments();
    }

    let mechanism = argument_text(&args[0])?;
    let initial_response = args.get(1).cloned();

    Ok(Command::Authenticate {
        tag,
        mechanism,
        initial_response,
    })
}

fn parse_login(tag: String, args: Vec<Argument>) -> std::io::Result<Command> {
    let args: [Argument; 2] = args.try_into().map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "Client command has invalid arguments\n",
        )
    })?;
    let [username, password] = args;

    Ok(Command::Login {
        tag,
        username,
        password,
    })
}

fn parse_select(tag: String, args: Vec<Argument>) -> std::io::Result<Command> {
    let args: [Argument; 1] = args.try_into().map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "Client command has invalid arguments\n",
        )
    })?;
    let [mailbox] = args;

    Ok(Command::Select { tag, mailbox })
}

fn parse_no_arg(
    tag: String,
    args: Vec<Argument>,
    build: impl FnOnce(String) -> Command,
) -> std::io::Result<Command> {
    if !args.is_empty() {
        return invalid_arguments();
    }

    Ok(build(tag))
}

fn invalid_arguments<T>() -> std::io::Result<T> {
    Err(Error::new(
        ErrorKind::InvalidInput,
        "Client command has invalid arguments\n",
    ))
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

fn argument_text(argument: &Argument) -> std::io::Result<String> {
    match argument {
        Argument::Atom(value) | Argument::Quoted(value) => Ok(value.to_string()),
        _ => invalid_arguments(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &str) -> Command {
        parse_command(&[CommandPart::Text(line.to_string())]).unwrap()
    }

    #[test]
    fn parse_command_trims_crlf() {
        let command = parse_line("A1 NOOP\r\n");

        assert_eq!(Command::Noop { tag: "A1".into() }, command);
    }

    #[test]
    fn parse_command_names_case_insensitively() {
        let command = parse_line("A1 capability\r\n");

        assert_eq!(Command::Capability { tag: "A1".into() }, command);
    }

    #[test]
    fn parse_mixed_case_command_names_case_insensitively() {
        let command = parse_line("A1 SeLeCt INBOX\r\n");

        assert_eq!(
            Command::Select {
                tag: "A1".into(),
                mailbox: Argument::Atom("INBOX".into()),
            },
            command
        );
    }

    #[test]
    fn parse_authenticate_preserves_initial_response() {
        let command = parse_line("A2 AUTHENTICATE XOAUTH2 token\r\n");

        assert_eq!(
            Command::Authenticate {
                tag: "A2".into(),
                mechanism: "XOAUTH2".into(),
                initial_response: Some(Argument::Atom("token".into()))
            },
            command
        );
    }

    #[test]
    fn parse_authenticate_allows_missing_initial_response() {
        let command = parse_line("A2 AUTHENTICATE XOAUTH2\r\n");

        assert_eq!(
            Command::Authenticate {
                tag: "A2".into(),
                mechanism: "XOAUTH2".into(),
                initial_response: None
            },
            command
        );
    }

    #[test]
    fn parse_command_preserves_typed_arguments_for_unknown_commands() {
        let command = parse_line(r#"A3 STORE 1 (\Seen "two words" NIL)"#);

        assert_eq!(
            Command::Unknown {
                tag: "A3".into(),
                name: "STORE".into(),
                args: vec![
                    Argument::Atom("1".into()),
                    Argument::List(vec![
                        Argument::Atom("\\Seen".into()),
                        Argument::Quoted("two words".into()),
                        Argument::Nil,
                    ])
                ]
            },
            command
        );
    }

    #[test]
    fn parse_select_requires_one_mailbox_argument() {
        let err = parse_command(&[CommandPart::Text("A1 SELECT\r\n".to_string())]).unwrap_err();

        assert_eq!(ErrorKind::InvalidInput, err.kind());
    }

    #[test]
    fn parse_literal_marker_detects_synchronizing_literals() {
        let marker = parse_literal_marker("A1 LOGIN {12}\r\n").unwrap();

        assert_eq!(Some((12, "A1 LOGIN ".to_string())), marker);
    }

    #[test]
    fn parse_command_preserves_binary_literal_payloads() {
        let command = parse_command(&[
            CommandPart::Text("A1 AUTHENTICATE XOAUTH2 ".to_string()),
            CommandPart::Literal(b"hello\r\n\xFFworld".to_vec()),
        ])
        .unwrap();

        assert_eq!(
            Command::Authenticate {
                tag: "A1".into(),
                mechanism: "XOAUTH2".into(),
                initial_response: Some(Argument::Literal(b"hello\r\n\xFFworld".to_vec())),
            },
            command
        );
    }
}
