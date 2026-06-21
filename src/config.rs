use crate::store::{FixtureMailStore, MailStore, SqliteMailStore};
use std::env;
use std::io::{Error, ErrorKind};
use std::sync::Arc;

const MAIL_STORE_ENV: &str = "MAIL_STORE";
const MAIL_DB_PATH_ENV: &str = "MAIL_DB_PATH";
const DEFAULT_MAIL_STORE: &str = "fixture";
const DEFAULT_MAIL_DB_PATH: &str = "/data/mail.sqlite3";

pub fn mail_store_from_env() -> std::io::Result<Arc<dyn MailStore>> {
    let store = env::var(MAIL_STORE_ENV).unwrap_or_else(|_| DEFAULT_MAIL_STORE.to_string());

    match store.as_str() {
        "fixture" => Ok(Arc::new(FixtureMailStore)),
        "sqlite" => {
            let path = mail_db_path_from_env();
            let store = SqliteMailStore::open(&path)
                .map_err(|err| Error::new(ErrorKind::Other, err.to_string()))?;

            Ok(Arc::new(store))
        }
        other => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Unsupported MAIL_STORE '{}'", other),
        )),
    }
}

fn mail_db_path_from_env() -> String {
    env::var(MAIL_DB_PATH_ENV).unwrap_or_else(|_| DEFAULT_MAIL_DB_PATH.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn unique_sqlite_path() -> String {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir()
            .join(format!("mail-store-{}.sqlite3", id))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn mail_store_defaults_to_fixture() {
        let _guard = lock_env();
        unsafe {
            env::remove_var(MAIL_STORE_ENV);
            env::remove_var(MAIL_DB_PATH_ENV);
        }

        let store = mail_store_from_env().unwrap();
        let selection = store.select_mailbox("INBOX").unwrap();

        assert_eq!(172, selection.exists);
        assert_eq!(4_392, selection.uid_next);
    }

    #[test]
    fn sqlite_mail_store_uses_default_database_path() {
        let _guard = lock_env();
        unsafe {
            env::remove_var(MAIL_DB_PATH_ENV);
        }

        assert_eq!(DEFAULT_MAIL_DB_PATH, mail_db_path_from_env());
    }

    #[test]
    fn invalid_mail_store_is_rejected() {
        let _guard = lock_env();
        unsafe {
            env::set_var(MAIL_STORE_ENV, "postgres");
            env::remove_var(MAIL_DB_PATH_ENV);
        }

        let err = match mail_store_from_env() {
            Ok(_) => panic!("expected invalid mail store to fail"),
            Err(err) => err,
        };

        assert_eq!(ErrorKind::InvalidInput, err.kind());
        assert_eq!("Unsupported MAIL_STORE 'postgres'", err.to_string());
    }

    #[test]
    fn mail_store_can_use_seeded_sqlite_database() {
        let _guard = lock_env();
        let path = unique_sqlite_path();
        unsafe {
            env::set_var(MAIL_STORE_ENV, "sqlite");
            env::set_var(MAIL_DB_PATH_ENV, &path);
        }

        let store = mail_store_from_env().unwrap();
        let selection = store.select_mailbox("inbox").unwrap();

        assert_eq!(172, selection.exists);
        assert_eq!(Some(12), selection.first_unseen);

        let _ = std::fs::remove_file(path);
    }
}
