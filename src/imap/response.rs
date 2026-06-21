use super::connection::{self, Connection};
use crate::store::{MailboxSelection, MessageFlag};

pub const GREETING: &str = "* OK IMAP4rev1 Service Ready\r\n";

pub fn tagged(tag: &str, status: &str, message: &str) -> String {
    format!(
        "{} {} {}\r\n",
        tag,
        status,
        message.trim_end_matches(&['\r', '\n'][..])
    )
}

pub fn untagged(message: &str) -> String {
    format!("* {}\r\n", message.trim_end_matches(&['\r', '\n'][..]))
}

pub async fn write_messages(
    connection: &Connection,
    messages: Vec<String>,
) -> std::io::Result<usize> {
    let refs = messages.iter().map(String::as_str).collect::<Vec<_>>();
    connection::write(connection, refs.as_slice()).await
}

pub async fn ok(connection: &Connection, tag: &str, message: &str) -> std::io::Result<usize> {
    write_messages(connection, vec![tagged(tag, "OK", message)]).await
}

pub async fn no(connection: &Connection, tag: &str, message: &str) -> std::io::Result<usize> {
    write_messages(connection, vec![tagged(tag, "NO", message)]).await
}

pub async fn bad(connection: &Connection, message: &str, tag: &str) -> std::io::Result<usize> {
    write_messages(connection, vec![tagged(tag, "BAD", message)]).await
}

pub async fn write_selection(
    connection: &Connection,
    id: &str,
    selection: &MailboxSelection,
) -> std::io::Result<usize> {
    let mut messages = vec![
        untagged(&format!("{} EXISTS", selection.exists)),
        untagged(&format!("{} RECENT", selection.recent)),
    ];

    if let Some(message) = selection.first_unseen {
        messages.push(untagged(&format!(
            "OK [UNSEEN {}] Message {} is first unseen",
            message, message
        )));
    }

    messages.extend([
        untagged(&format!(
            "OK [UIDVALIDITY {}] UIDs valid",
            selection.uid_validity
        )),
        untagged(&format!(
            "OK [UIDNEXT {}] Predicted next UID",
            selection.uid_next
        )),
        untagged(&format!("FLAGS ({})", format_flags(&selection.flags))),
        untagged(&format!(
            "OK [PERMANENTFLAGS ({})] Limited",
            format_flags(&selection.permanent_flags)
        )),
        tagged(id, "OK", "[READ-WRITE] SELECT completed"),
    ]);

    write_messages(connection, messages).await
}

fn format_flags(flags: &[MessageFlag]) -> String {
    flags
        .iter()
        .map(MessageFlag::as_imap)
        .collect::<Vec<_>>()
        .join(" ")
}
