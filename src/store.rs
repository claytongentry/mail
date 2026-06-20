use std::fmt;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailStoreError {
    MailboxNotFound(String),
}

impl fmt::Display for MailStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MailStoreError::MailboxNotFound(_mailbox) => {
                write!(formatter, "Mailbox does not exist")
            }
        }
    }
}

impl std::error::Error for MailStoreError {}

pub struct FixtureMailStore;

impl MailStore for FixtureMailStore {
    fn select_mailbox(&self, mailbox: &str) -> MailStoreResult<MailboxSelection> {
        if !mailbox.eq_ignore_ascii_case("INBOX") {
            return Err(MailStoreError::MailboxNotFound(mailbox.to_string()));
        }

        Ok(MailboxSelection {
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
        })
    }
}
