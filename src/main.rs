use async_std::net::TcpListener;
use futures::stream::StreamExt;
use std::env;
use std::io::{Error, ErrorKind};
mod connection;
mod oauth2;
use connection::{Argument, Command, Connection};

async fn bad(connection: &Connection, message: &str, tag: &String) -> std::io::Result<usize> {
    let response = tag.to_string() + &(" BAD ".to_string()) + &message.to_string();
    connection::write(connection, &[&response]).await
}

/**
 ****************************************************************
 * Begin response implementations
 ****************************************************************
 */

/**
 * https://tools.ietf.org/html/rfc2060#section-6.2.1
 */
async fn authenticate(
    connection: &mut Connection,
    id: &String,
    args: &[Argument],
) -> std::io::Result<usize> {
    if args.len() != 2 {
        return bad(connection, &"Arguments invalid\n", id).await;
    }

    let mechanism = match args[0].as_utf8() {
        Some(mechanism) => mechanism,
        None => return bad(connection, &"Arguments invalid\n", id).await,
    };
    let token = match args[1].as_utf8() {
        Some(token) => token,
        None => return bad(connection, &"Arguments invalid\n", id).await,
    };

    match mechanism {
        "XOAUTH2" => match oauth2::authenticate(token) {
            Ok(_claims) => {
                connection::set_authenticated_state(connection);
                connection::write(connection, &["OK SASL authentication successful\n"]).await
            }
            _err => {
                connection::write(
                    connection,
                    &[&(id.to_string() + " NO Invalid credentials\n")],
                )
                .await
            }
        },
        _other => {
            connection::write(
                connection,
                &[&(id.to_string() + " NO Unsupported authentication mechanism\n")],
            )
            .await
        }
    }
}

async fn capability(connection: &Connection, id: &String) -> std::io::Result<usize> {
    connection::write(
        connection,
        &[
            "* CAPABILITY IMAP4rev1 AUTH=XOAUTH2 LOGINDISABLED\n",
            &(id.to_string() + " OK CAPABILITY completed\n"),
        ],
    )
    .await
}

async fn login(connection: &Connection, id: &String) -> std::io::Result<usize> {
    connection::write(
        connection,
        &[&(id.to_string() + " NO Login is disabled.\n")],
    )
    .await
}

async fn logout(connection: &mut Connection, id: &String) -> std::io::Result<usize> {
    connection::set_logout_state(connection);
    connection::write(
        connection,
        &[
            "* BYE IMAPrev1 Server logging out\n",
            &(id.to_string() + " OK LOGOUT completed\n"),
        ],
    )
    .await
}

/**
* TODO:
* Since any command can return a status update as untagged data, the
    NOOP command can be used as a periodic poll for new messages or
    message status updates during a period of inactivity.  The NOOP
    command can also be used to reset any inactivity autologout timer
    on the server.
* https://tools.ietf.org/html/rfc2060#section-6.1.2
*/
async fn noop(connection: &Connection, id: &String) -> std::io::Result<usize> {
    connection::write(connection, &[&(id.to_string() + " OK NOOP completed\n")]).await
}

async fn select(connection: &Connection, id: &String) -> std::io::Result<usize> {
    connection::write(
        connection,
        &[
            "* 172 EXISTS\n",
            "* 1 RECENT\n",
            "* OK [UNSEEN 12] Message 12 is first unseen\n",
            "* OK [UIDVALIDITY 3857529045] UIDs valid\n",
            "* OK [UIDNEXT 4392] Predicted next UID\n",
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\n",
            "* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\n",
            &(id.to_string() + "OK [READ-WRITE] SELECT completed\n"),
        ],
    )
    .await
}

/**
 ****************************************************************
 * End response implementations
 ****************************************************************
 */

async fn handle_command(command: &Command, connection: &mut Connection) -> std::io::Result<usize> {
    match command.name.as_str() {
        "AUTHENTICATE" => authenticate(connection, &command.tag, command.args.as_slice()).await,
        "CAPABILITY" => capability(connection, &command.tag).await,
        "LOGIN" => login(connection, &command.tag).await,
        "LOGOUT" => logout(connection, &command.tag).await,
        "NOOP" => noop(connection, &command.tag).await,
        "SELECT" => select(connection, &command.tag).await,
        _other => {
            let message = command.name.to_string() + " is not a valid command.\n";
            Err(Error::new(ErrorKind::InvalidInput, message))
        }
    }
}

async fn handle_connection(connection: &mut Connection) {
    loop {
        match connection::read_command(connection).await {
            Ok(command) => {
                let tag = command.tag.to_string();
                let name = command.name.to_string();
                let _ = match handle_command(&command, connection).await {
                    Ok(val) => Ok(val),
                    Err(err) => match err.kind() {
                        ErrorKind::BrokenPipe => break,
                        ErrorKind::InvalidInput => bad(&connection, &err.to_string(), &tag).await,
                        _other => panic!("Error!, {:?}", err),
                    },
                };

                // Can use this space to close connections as needed
                match name.as_str() {
                    "LOGOUT" => break,
                    other => other,
                };
            }
            Err(err) => {
                let tag = "*".to_string();
                let msg = &err.to_string();
                let _ = bad(&connection, msg, &tag).await;
            }
        }
    }
}

#[async_std::main]
async fn main() {
    let bind_addr = env::var("IMAP_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:1143".to_string());
    println!("IMAPrev1 listening on {}...", bind_addr);

    /*
     * IMAPrev1 servers listen on port 143
     * https://tools.ietf.org/html/rfc2060#section-2.1
     */
    let listener = TcpListener::bind(bind_addr.as_str()).await.unwrap();

    // accept connections concurrently
    listener
        .incoming()
        .for_each_concurrent(/* limit */ None, |stream| async move {
            let stream = stream.unwrap();
            let mut conn = connection::new(stream);
            handle_connection(&mut conn).await;
        })
        .await
}
