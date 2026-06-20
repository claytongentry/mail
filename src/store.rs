use rusqlite::{params, Connection};
use std::convert::TryFrom;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

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

fn fixture_selection() -> MailboxSelection {
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

pub struct FixtureMailStore;

impl MailStore for FixtureMailStore {
    fn select_mailbox(&self, mailbox: &str) -> MailStoreResult<MailboxSelection> {
        if !mailbox.eq_ignore_ascii_case("INBOX") {
            return Err(MailStoreError::MailboxNotFound(mailbox.to_string()));
        }

        Ok(fixture_selection())
    }
}

pub struct SqliteMailStore {
    connection: Mutex<Connection>,
}

impl SqliteMailStore {
    pub fn open(path: &str) -> MailStoreResult<SqliteMailStore> {
        let connection = Connection::open(path).map_err(sqlite_error)?;
        let store = SqliteMailStore {
            connection: Mutex::new(connection),
        };
        store.initialize()?;
        Ok(store)
    }

    #[cfg(test)]
    fn open_in_memory() -> MailStoreResult<SqliteMailStore> {
        let connection = Connection::open_in_memory().map_err(sqlite_error)?;
        let store = SqliteMailStore {
            connection: Mutex::new(connection),
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> MailStoreResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(sqlite_error)?;

        transaction
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS mailboxes (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                    exists_count INTEGER NOT NULL,
                    recent_count INTEGER NOT NULL,
                    first_unseen INTEGER,
                    uid_validity INTEGER NOT NULL,
                    uid_next INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS mailbox_flags (
                    mailbox_id INTEGER NOT NULL,
                    flag TEXT NOT NULL,
                    permanent INTEGER NOT NULL,
                    sort_order INTEGER NOT NULL,
                    PRIMARY KEY (mailbox_id, permanent, sort_order),
                    FOREIGN KEY (mailbox_id) REFERENCES mailboxes(id) ON DELETE CASCADE
                );
                ",
            )
            .map_err(sqlite_error)?;

        seed_inbox(&transaction)?;
        transaction.commit().map_err(sqlite_error)
    }

    fn connection(&self) -> MailStoreResult<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| {
            MailStoreError::Storage("SQLite connection lock is poisoned".to_string())
        })
    }
}

impl MailStore for SqliteMailStore {
    fn select_mailbox(&self, mailbox: &str) -> MailStoreResult<MailboxSelection> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "
                SELECT id, exists_count, recent_count, first_unseen, uid_validity, uid_next
                FROM mailboxes
                WHERE name = ?1 COLLATE NOCASE
                ",
            )
            .map_err(sqlite_error)?;

        let row = statement.query_row(params![mailbox], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        });

        let (mailbox_id, exists, recent, first_unseen, uid_validity, uid_next) = match row {
            Ok(values) => values,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(MailStoreError::MailboxNotFound(mailbox.to_string()));
            }
            Err(err) => return Err(sqlite_error(err)),
        };

        Ok(MailboxSelection {
            exists: to_u32(exists, "exists_count")?,
            recent: to_u32(recent, "recent_count")?,
            first_unseen: match first_unseen {
                Some(value) => Some(to_u32(value, "first_unseen")?),
                None => None,
            },
            uid_validity: to_u32(uid_validity, "uid_validity")?,
            uid_next: to_u32(uid_next, "uid_next")?,
            flags: load_flags(&connection, mailbox_id, false)?,
            permanent_flags: load_flags(&connection, mailbox_id, true)?,
        })
    }
}

fn seed_inbox(transaction: &rusqlite::Transaction<'_>) -> MailStoreResult<()> {
    let selection = fixture_selection();

    transaction
        .execute(
            "
            INSERT OR IGNORE INTO mailboxes
                (name, exists_count, recent_count, first_unseen, uid_validity, uid_next)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ",
            params![
                "INBOX",
                i64::from(selection.exists),
                i64::from(selection.recent),
                selection.first_unseen.map(i64::from),
                i64::from(selection.uid_validity),
                i64::from(selection.uid_next)
            ],
        )
        .map_err(sqlite_error)?;

    let mailbox_id = transaction
        .query_row(
            "SELECT id FROM mailboxes WHERE name = ?1 COLLATE NOCASE",
            params!["INBOX"],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?;

    seed_flags(transaction, mailbox_id, false, &selection.flags)?;
    seed_flags(
        transaction,
        mailbox_id,
        true,
        &selection.permanent_flags,
    )
}

fn seed_flags(
    transaction: &rusqlite::Transaction<'_>,
    mailbox_id: i64,
    permanent: bool,
    flags: &[MessageFlag],
) -> MailStoreResult<()> {
    for (index, flag) in flags.iter().enumerate() {
        transaction
            .execute(
                "
                INSERT OR IGNORE INTO mailbox_flags
                    (mailbox_id, flag, permanent, sort_order)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![mailbox_id, flag.as_imap(), permanent, index as i64],
            )
            .map_err(sqlite_error)?;
    }

    Ok(())
}

fn load_flags(
    connection: &Connection,
    mailbox_id: i64,
    permanent: bool,
) -> MailStoreResult<Vec<MessageFlag>> {
    let mut statement = connection
        .prepare(
            "
            SELECT flag
            FROM mailbox_flags
            WHERE mailbox_id = ?1 AND permanent = ?2
            ORDER BY sort_order
            ",
        )
        .map_err(sqlite_error)?;

    let rows = statement
        .query_map(params![mailbox_id, permanent], |row| row.get::<_, String>(0))
        .map_err(sqlite_error)?;
    let mut flags = Vec::new();

    for row in rows {
        flags.push(MessageFlag::try_from_imap(&row.map_err(sqlite_error)?)?);
    }

    Ok(flags)
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

fn to_u32(value: i64, field: &str) -> MailStoreResult<u32> {
    u32::try_from(value).map_err(|_| {
        MailStoreError::Storage(format!("{} value is outside u32 range", field))
    })
}

fn sqlite_error(err: rusqlite::Error) -> MailStoreError {
    MailStoreError::Storage(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_inbox_flag(store: &SqliteMailStore, flag: &str, sort_order: i64) {
        let connection = store.connection().unwrap();
        let mailbox_id = connection
            .query_row(
                "SELECT id FROM mailboxes WHERE name = ?1 COLLATE NOCASE",
                params!["INBOX"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        connection
            .execute(
                "
                INSERT INTO mailbox_flags (mailbox_id, flag, permanent, sort_order)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![mailbox_id, flag, false, sort_order],
            )
            .unwrap();
    }

    #[test]
    fn sqlite_store_seeds_inbox_on_initialization() {
        let store = SqliteMailStore::open_in_memory().unwrap();

        assert_eq!(fixture_selection(), store.select_mailbox("INBOX").unwrap());
    }

    #[test]
    fn sqlite_store_selects_mailboxes_case_insensitively() {
        let store = SqliteMailStore::open_in_memory().unwrap();

        assert_eq!(fixture_selection(), store.select_mailbox("inbox").unwrap());
    }

    #[test]
    fn sqlite_store_returns_not_found_for_unknown_mailbox() {
        let store = SqliteMailStore::open_in_memory().unwrap();
        let err = store.select_mailbox("Archive").unwrap_err();

        assert_eq!(MailStoreError::MailboxNotFound("Archive".to_string()), err);
    }

    #[test]
    fn sqlite_store_does_not_duplicate_inbox_seed() {
        let store = SqliteMailStore::open_in_memory().unwrap();

        store.initialize().unwrap();

        let connection = store.connection().unwrap();
        let count = connection
            .query_row("SELECT COUNT(*) FROM mailboxes", [], |row| row.get::<_, i64>(0))
            .unwrap();

        assert_eq!(1, count);
    }

    #[test]
    fn sqlite_store_round_trips_custom_flags() {
        let store = SqliteMailStore::open_in_memory().unwrap();

        insert_inbox_flag(&store, "$Forwarded", 99);

        let selection = store.select_mailbox("INBOX").unwrap();

        assert!(selection
            .flags
            .contains(&MessageFlag::Custom("$Forwarded".to_string())));
    }

    #[test]
    fn sqlite_store_rejects_flags_that_cannot_be_imap_atoms() {
        let store = SqliteMailStore::open_in_memory().unwrap();

        insert_inbox_flag(&store, "bad flag\r\n* OK injected", 99);

        let err = store.select_mailbox("INBOX").unwrap_err();

        assert_eq!(
            MailStoreError::Storage("Invalid IMAP flag atom in SQLite store".to_string()),
            err
        );
    }

    #[test]
    fn sqlite_store_rejects_parenthesized_flags() {
        let store = SqliteMailStore::open_in_memory().unwrap();

        insert_inbox_flag(&store, "bad(flag)", 99);

        let err = store.select_mailbox("INBOX").unwrap_err();

        assert_eq!(
            MailStoreError::Storage("Invalid IMAP flag atom in SQLite store".to_string()),
            err
        );
    }
}
