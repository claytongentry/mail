use super::command::{Argument, Command};
use super::connection::{self, Connection, ConnectionState};
use super::response;
use crate::auth;
use crate::store::MailStore;
use std::io::{Error, ErrorKind};

fn write_done(result: std::io::Result<usize>) -> std::io::Result<()> {
    result.map(|_| ())
}

async fn authenticate(
    connection: &mut Connection,
    id: &str,
    mechanism: &str,
    initial_response: &Option<Argument>,
) -> std::io::Result<usize> {
    if mechanism.eq_ignore_ascii_case("XOAUTH2") {
        let token = match auth::xoauth2::bearer_token(initial_response) {
            Ok(token) => token,
            Err(err) => return response::bad(connection, &err.to_string(), id).await,
        };

        match auth::jwt::authenticate(&token) {
            Ok(_claims) => {
                connection::set_authenticated_state(connection);
                response::ok(connection, id, "SASL authentication successful").await
            }
            _err => response::no(connection, id, "Invalid credentials").await,
        }
    } else {
        response::no(connection, id, "Unsupported authentication mechanism").await
    }
}

async fn capability(connection: &Connection, id: &str) -> std::io::Result<usize> {
    response::write_messages(
        connection,
        vec![
            response::untagged("CAPABILITY IMAP4rev1 AUTH=XOAUTH2 LOGINDISABLED SASL-IR"),
            response::tagged(id, "OK", "CAPABILITY completed"),
        ],
    )
    .await
}

async fn login(connection: &Connection, id: &str) -> std::io::Result<usize> {
    response::no(connection, id, "Login is disabled.").await
}

async fn logout(connection: &mut Connection, id: &str) -> std::io::Result<usize> {
    connection::set_logout_state(connection);
    response::write_messages(
        connection,
        vec![
            response::untagged("BYE IMAPrev1 Server logging out"),
            response::tagged(id, "OK", "LOGOUT completed"),
        ],
    )
    .await
}

async fn noop(connection: &Connection, id: &str) -> std::io::Result<usize> {
    response::ok(connection, id, "NOOP completed").await
}

async fn select(
    connection: &Connection,
    id: &str,
    mailbox: &Argument,
    store: &(impl MailStore + ?Sized),
) -> std::io::Result<usize> {
    let mailbox = match mailbox.as_utf8() {
        Some(mailbox) => mailbox,
        None => {
            return response::bad(connection, "Client command has invalid arguments", id).await;
        }
    };
    let selection = match store.select_mailbox(mailbox) {
        Ok(selection) => selection,
        Err(err) => return response::no(connection, id, &err.to_string()).await,
    };

    response::write_selection(connection, id, &selection).await
}

async fn handle_command(
    command: &Command,
    connection: &mut Connection,
    store: &(impl MailStore + ?Sized),
) -> std::io::Result<()> {
    let state = connection::state(connection);

    if !command_is_valid_for_state(command, state) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Command {} is not valid in {} state",
                command.name(),
                state.as_imap_name()
            ),
        ));
    }

    match command {
        Command::Authenticate {
            tag,
            mechanism,
            initial_response,
        } => write_done(authenticate(connection, tag, mechanism, initial_response).await),
        Command::Capability { tag } => write_done(capability(connection, tag).await),
        Command::Login { tag, .. } => write_done(login(connection, tag).await),
        Command::Logout { tag } => write_done(logout(connection, tag).await),
        Command::Noop { tag } => write_done(noop(connection, tag).await),
        Command::Select { tag, mailbox } => {
            write_done(select(connection, tag, mailbox, store).await)
        }
        Command::Unknown { name, .. } => {
            let message = name.to_string() + " is not a valid command.";
            Err(Error::new(ErrorKind::InvalidInput, message))
        }
    }
}

fn command_is_valid_for_state(command: &Command, state: ConnectionState) -> bool {
    match state {
        ConnectionState::NotAuthenticated => matches!(
            command,
            Command::Authenticate { .. }
                | Command::Capability { .. }
                | Command::Login { .. }
                | Command::Logout { .. }
                | Command::Noop { .. }
                | Command::Unknown { .. }
        ),
        ConnectionState::Authenticated => matches!(
            command,
            Command::Capability { .. }
                | Command::Logout { .. }
                | Command::Noop { .. }
                | Command::Select { .. }
                | Command::Unknown { .. }
        ),
        ConnectionState::Logout => false,
    }
}

pub async fn handle_connection(
    connection: &mut Connection,
    store: &(impl MailStore + ?Sized),
) {
    if connection::write(connection, &[response::GREETING])
        .await
        .is_err()
    {
        return;
    }

    loop {
        match connection::read_command(connection).await {
            Ok(command) => {
                let tag = command.tag().to_string();
                match handle_command(&command, connection, store).await {
                    Ok(()) => {
                        if connection::state(connection) == ConnectionState::Logout {
                            break;
                        }
                    }
                    Err(err) => match err.kind() {
                        ErrorKind::BrokenPipe => break,
                        ErrorKind::InvalidInput => {
                            let _ = response::bad(connection, &err.to_string(), &tag).await;
                        }
                        _other => {
                            eprintln!("Connection error: {}", err);
                            break;
                        }
                    },
                }
            }
            Err(err) => {
                if err.kind() == ErrorKind::BrokenPipe {
                    break;
                }

                let tag = "*".to_string();
                let msg = &err.to_string();
                let _ = response::bad(connection, msg, &tag).await;
            }
        }
    }
}
