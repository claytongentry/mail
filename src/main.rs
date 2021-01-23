use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::str;

fn handle_client(mut stream: TcpStream) {
    let mut buffer = [0; 10];
    let _len = stream.read(&mut buffer);
    let string_value = str::from_utf8(&mut buffer).unwrap();
    println!("Server got bytes: {}", string_value);
}

fn main() -> std::io::Result<()> {
    /*
     * IMAPrev1 servers listen on port 143
     * https://tools.ietf.org/html/rfc2060#section-2.1
     */
    let listener = TcpListener::bind("127.0.0.1:1143")?;

    // accept connections and process them serially
    for stream in listener.incoming() {
        handle_client(stream?);
    }
    Ok(())
}
