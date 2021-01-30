use async_std::net::TcpListener;
use futures::stream::StreamExt;
use std::io::{Error, ErrorKind};
mod connection;
mod oauth2;
use connection::Connection;

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
    args: Vec<String>,
) -> std::io::Result<usize> {
    if args.len() != 2 {
        return bad(connection, &"Arguments invalid\n", id).await;
    }

    let mechanism = &args[0];
    let token = &args[1];

    match mechanism.as_str() {
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
    connection::write(connection, &[&(id.to_string() + " NO Login is disabled.\n")]).await
}

async fn logout(connection: &Connection, id: &String) -> std::io::Result<usize> {
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

/**
 ****************************************************************
 * End response implementations
 ****************************************************************
 */

async fn handle_command(
    command: &String,
    id: &String,
    args: Vec<String>,
    connection: &mut Connection,
) -> std::io::Result<usize> {
    match command.as_str() {
        "AUTHENTICATE" => authenticate(connection, id, args).await,
        "CAPABILITY" => capability(&connection, id).await,
        "LOGIN" => login(connection, id).await,
        "LOGOUT" => logout(&connection, id).await,
        "NOOP" => noop(&connection, id).await,
        _other => {
            let message = command.to_string() + " is not a valid command.\n";
            Err(Error::new(ErrorKind::InvalidInput, message))
        }
    }
}

async fn handle_connection(connection: &mut Connection) {
    loop {
        match connection::read_command(connection).await {
            Ok((command, tag, args)) => {
                let _ = match handle_command(&command, &tag, args, connection).await {
                    Ok(val) => Ok(val),
                    Err(err) => match err.kind() {
                        ErrorKind::BrokenPipe => break,
                        ErrorKind::InvalidInput => bad(&connection, &err.to_string(), &tag).await,
                        _other => panic!("Error!, {:?}", err),
                    },
                };

                // Can use this space to close connections as needed
                match command.as_str() {
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
    println!("IMAPrev1 listening on 1143...");

    /*
     * IMAPrev1 servers listen on port 143
     * https://tools.ietf.org/html/rfc2060#section-2.1
     */
    let listener = TcpListener::bind("127.0.0.1:1143").await.unwrap();

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
