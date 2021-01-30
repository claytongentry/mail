use async_std::io::prelude::*;
use async_std::net::TcpStream;
use futures::stream::{self, StreamExt};
use std::io::{Error, ErrorKind};

enum ConnectionState {
    NOTAUTHENTICATED,
    AUTHENTICATED,
}

pub struct Connection {
    state: ConnectionState,
    stream: TcpStream,
}

async fn read(connection: &Connection) -> [u8; 1024] {
    let mut buffer = [0; 1024];
    let mut stream = &connection.stream;
    let _ = stream.read(&mut buffer).await;
    buffer
}

pub fn new(stream: TcpStream) -> Connection {
    let state = ConnectionState::NOTAUTHENTICATED;
    Connection { state, stream }
}

pub async fn write(connection: &Connection, messages: &[&str]) -> std::io::Result<usize> {
    let mut tcpstream = &connection.stream;
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

pub async fn read_command(
    connection: &Connection,
) -> std::io::Result<(String, String, Vec<String>)> {
    let buffer = read(connection).await;

    let command = String::from_utf8(buffer.to_vec()).unwrap();
    let command = command.trim_matches(char::from(0));
    let command = command.trim_matches(char::from(10));

    let v: Vec<&str> = command.splitn(3, ' ').collect();

    let length = v.len();
    if length > 3 || length < 2 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Client commands should have at least an identifier and a valid IMAPrev1 command\n",
        ));
    }

    let command = v[1].to_string();
    let tag = v[0].to_string();
    let mut args: Vec<String> = Vec::new();

    if length == 3 {
        args = v[2].split(' ').map(String::from).collect();
    }

    return Ok((command, tag, args));
}

pub fn set_authenticated_state(connection: &mut Connection) {
    set_state(connection, ConnectionState::AUTHENTICATED);
}

fn set_state(connection: &mut Connection, state: ConnectionState) {
    connection.state = state;
}