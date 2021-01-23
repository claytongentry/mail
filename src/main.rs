use async_std::io::prelude::*;
use async_std::net::{TcpListener, TcpStream};
use futures::stream::StreamExt;
use std::str;

async fn handle_client(mut stream: TcpStream) {
    let mut buffer = [0; 1024];
    stream.read(&mut buffer).await.unwrap();

    let string_value = str::from_utf8(&mut buffer).unwrap();
    println!("Server got bytes: {}", string_value);

    let response = "ack\n";
    stream.write(response.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
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
