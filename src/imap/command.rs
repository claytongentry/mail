#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Authenticate {
        tag: String,
        mechanism: String,
        initial_response: Option<Argument>,
    },
    Capability {
        tag: String,
    },
    Login {
        tag: String,
        username: Argument,
        password: Argument,
    },
    Logout {
        tag: String,
    },
    Noop {
        tag: String,
    },
    Select {
        tag: String,
        mailbox: Argument,
    },
    Unknown {
        tag: String,
        name: String,
        args: Vec<Argument>,
    },
}

impl Command {
    pub fn tag(&self) -> &str {
        match self {
            Command::Authenticate { tag, .. }
            | Command::Capability { tag }
            | Command::Login { tag, .. }
            | Command::Logout { tag }
            | Command::Noop { tag }
            | Command::Select { tag, .. }
            | Command::Unknown { tag, .. } => tag,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Command::Authenticate { .. } => "AUTHENTICATE",
            Command::Capability { .. } => "CAPABILITY",
            Command::Login { .. } => "LOGIN",
            Command::Logout { .. } => "LOGOUT",
            Command::Noop { .. } => "NOOP",
            Command::Select { .. } => "SELECT",
            Command::Unknown { name, .. } => name,
        }
    }
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

#[derive(Debug, PartialEq, Eq)]
pub enum CommandPart {
    Text(String),
    Literal(Vec<u8>),
}
