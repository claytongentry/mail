use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use futures::stream::StreamExt;
use std::io::ErrorKind;
use std::string::String;

async fn handle_client(mut stream: TcpStream) {
    loop {
        let mut buffer = [0; 1024];
        stream.read(&mut buffer).await.unwrap();

        let command = String::from_utf8(buffer.to_vec()).unwrap();
        let command = command.trim_matches(char::from(0));

        match command {
            "CAPABILITY\n" => {
                println!("Got capability! {:?}", command);
            }
            _other => {
                println!("Huh? {}", command);
            }
        };

        let response = "ack\n";
        let result = stream.write(response.as_bytes()).await;
        match result {
            Ok(val) => val,
            Err(err) => match err.kind() {
                ErrorKind::BrokenPipe => break,
                _other => panic!("Error writing to TCP connection: {:?}", err),
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
        .for_each_concurrent(/* limit */ None, |tcpstream| async move {
            let tcpstream = tcpstream.unwrap();
            handle_client(tcpstream).await;
        })
        .await
}
