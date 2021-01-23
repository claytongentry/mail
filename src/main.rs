use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use futures::stream::{self, StreamExt};
use std::io::{Error, ErrorKind};
use std::string::String;

async fn write(mut tcpstream: &TcpStream, messages: &[&str]) -> std::io::Result<usize> {
    stream::iter(messages)
        .fold(Ok(0), |acc, msg| async move {
            match acc {
                Err(err) => Err(err),
                Ok(bytes) => match tcpstream.write(msg.as_bytes()).await {
                    Err(err) => Err(err),
                    Ok(new_bytes) => Ok(bytes + new_bytes),
                },
            }
        })
        .await
}

async fn bad(stream: &TcpStream, message: &str, tag: &str) -> std::io::Result<usize> {
    let response = tag.to_string() + &(" BAD ".to_string()) + &message.to_string();
    write(stream, &[&response]).await
}

/**
 ****************************************************************
 * Begin response implementations
 ****************************************************************
 */

async fn capability(stream: &TcpStream, id: &str) -> std::io::Result<usize> {
    write(
        stream,
        &[
            "* CAPABILITY IMAP4rev1\n",
            &(id.to_string() + " OK CAPABILITY completed\n"),
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
async fn noop(stream: &TcpStream, id: &str) -> std::io::Result<usize> {
    write(stream, &[&(id.to_string() + " OK NOOP completed\n")]).await
}

/**
 ****************************************************************
 * End response implementations
 ****************************************************************
 */

fn parse_client_command(command: &str) -> std::io::Result<(&str, &str)> {
    let v: Vec<&str> = command.splitn(2, ' ').collect();

    if v.len() < 2 {
        let error = Error::new(
            ErrorKind::InvalidInput,
            "Client commands should have at least an identifier and a valid IMAPrev1 command\n",
        );
        return Err(error);
    }

    Ok((v[0], v[1]))
}

async fn handle_command(command: &str, id: &str, stream: &TcpStream) -> std::io::Result<usize> {
    match command {
        "CAPABILITY" => capability(stream, id).await,
        "NOOP" => noop(stream, id).await,
        _other => {
            let message = command.to_string() + " is not a valid command.\n";
            Err(Error::new(ErrorKind::InvalidInput, message))
        }
    }
}

async fn handle_client(mut stream: TcpStream) {
    loop {
        let mut buffer = [0; 1024];
        match stream.read(&mut buffer).await {
            Ok(val) => val,
            Err(_) => break,
        };

        let command = String::from_utf8(buffer.to_vec()).unwrap();
        let command = command.trim_matches(char::from(0));
        let command = command.trim_matches(char::from(10));

        let _result = match parse_client_command(command) {
            Ok((tag, command)) => match handle_command(command, tag, &stream).await {
                Ok(val) => Ok(val),
                Err(err) => match err.kind() {
                    ErrorKind::BrokenPipe => break,
                    ErrorKind::InvalidInput => bad(&stream, &err.to_string(), tag).await,
                    _other => panic!("Error!, {:?}", err),
                },
            },
            Err(err) => bad(&stream, &err.to_string(), "*").await,
        };

        stream.flush().await.unwrap();
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
            handle_client(stream).await;
        })
        .await
}
