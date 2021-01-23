use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use futures::stream::{self, StreamExt};
use std::io::ErrorKind;
use std::string::String;

async fn write(mut tcpstream: &TcpStream, messages: &[&str]) -> std::io::Result<usize> {
    stream::iter(messages)
        .fold(
            Ok(0),
            |acc: std::result::Result<usize, std::io::Error>, msg| async move {
                match acc {
                    Err(err) => Err(err),
                    Ok(bytes) => {
                        let result = tcpstream.write(msg.as_bytes()).await;
                        match result {
                            Err(err) => Err(err),
                            Ok(new_bytes) => Ok(bytes + new_bytes),
                        }
                    }
                }
            },
        )
        .await
}

async fn capability(stream: &TcpStream) -> std::io::Result<usize> {
    write(
        stream,
        &["* CAPABILITY IMAP4rev1\n", "abcd OK CAPABILITY completed\n"],
    )
    .await
}

async fn handle_client(mut stream: TcpStream) {
    loop {
        let mut buffer = [0; 1024];
        stream.read(&mut buffer).await.unwrap();

        let command = String::from_utf8(buffer.to_vec()).unwrap();
        let command = command.trim_matches(char::from(0));
        let command = command.trim_matches(char::from(10));

        let result = match command {
            "CAPABILITY" => capability(&stream).await,
            _other => {
                println!("Huh? {}", command);
                Ok(0)
            }
        };

        match result {
            Ok(val) => val,
            Err(err) => match err.kind() {
                ErrorKind::BrokenPipe => break,
                _other => panic!("Error!, {:?}", err),
            },
        };

        stream.flush().await.unwrap();
    }
}

#[async_std::main]
async fn main() {
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
