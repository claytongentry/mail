use super::{fixture_selection, MailStore, MailStoreError, MailStoreResult, MailboxSelection};

pub struct FixtureMailStore;

impl MailStore for FixtureMailStore {
    fn select_mailbox(&self, mailbox: &str) -> MailStoreResult<MailboxSelection> {
        if !mailbox.eq_ignore_ascii_case("INBOX") {
            return Err(MailStoreError::MailboxNotFound(mailbox.to_string()));
        }

        Ok(fixture_selection())
    }
}
