mod fixture;
mod sqlite;

use std::fmt;

pub use fixture::FixtureMailStore;
pub use sqlite::SqliteMailStore;

pub type MailStoreResult<T> = Result<T, MailStoreError>;

pub trait MailStore: Send + Sync {
    fn select_mailbox(&self, mailbox: &str) -> MailStoreResult<MailboxSelection>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxSelection {
    pub exists: u32,
    pub recent: u32,
    pub first_unseen: Option<u32>,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub flags: Vec<MessageFlag>,
    pub permanent_flags: Vec<MessageFlag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageFlag {
    Answered,
    Flagged,
    Deleted,
    Seen,
    Draft,
    Custom(String),
    Wildcard,
}

impl MessageFlag {
    pub fn as_imap(&self) -> String {
        match self {
            MessageFlag::Answered => "\\Answered".to_string(),
            MessageFlag::Flagged => "\\Flagged".to_string(),
            MessageFlag::Deleted => "\\Deleted".to_string(),
            MessageFlag::Seen => "\\Seen".to_string(),
            MessageFlag::Draft => "\\Draft".to_string(),
            MessageFlag::Custom(value) => value.to_string(),
            MessageFlag::Wildcard => "\\*".to_string(),
        }
    }

    fn try_from_imap(value: &str) -> MailStoreResult<MessageFlag> {
        match value {
            "\\Answered" => Ok(MessageFlag::Answered),
            "\\Flagged" => Ok(MessageFlag::Flagged),
            "\\Deleted" => Ok(MessageFlag::Deleted),
            "\\Seen" => Ok(MessageFlag::Seen),
            "\\Draft" => Ok(MessageFlag::Draft),
            "\\*" => Ok(MessageFlag::Wildcard),
            other if is_valid_flag_atom(other) => Ok(MessageFlag::Custom(other.to_string())),
            _other => Err(MailStoreError::Storage(
                "Invalid IMAP flag atom in SQLite store".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailStoreError {
    MailboxNotFound(String),
    Storage(String),
}

impl fmt::Display for MailStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MailStoreError::MailboxNotFound(_mailbox) => {
                write!(formatter, "Mailbox does not exist")
            }
            MailStoreError::Storage(message) => {
                write!(formatter, "Mail store error: {}", message)
            }
        }
    }
}

impl std::error::Error for MailStoreError {}

pub(crate) fn fixture_selection() -> MailboxSelection {
    MailboxSelection {
        exists: 172,
        recent: 1,
        first_unseen: Some(12),
        uid_validity: 3_857_529_045,
        uid_next: 4_392,
        flags: vec![
            MessageFlag::Answered,
            MessageFlag::Flagged,
            MessageFlag::Deleted,
            MessageFlag::Seen,
            MessageFlag::Draft,
        ],
        permanent_flags: vec![
            MessageFlag::Deleted,
            MessageFlag::Seen,
            MessageFlag::Wildcard,
        ],
    }
}

fn is_valid_flag_atom(value: &str) -> bool {
    !value.is_empty()
        && value.is_ascii()
        && !value.bytes().any(|byte| {
            byte.is_ascii_control()
                || byte.is_ascii_whitespace()
                || matches!(byte, b'(' | b')' | b'{' | b'%' | b'*' | b'"' | b'\\' | b']')
        })
}
