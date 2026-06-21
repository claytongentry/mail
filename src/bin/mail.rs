use async_std::net::TcpListener;
use futures::stream::StreamExt;
use mail::config;
use mail::imap::{connection, session};
use std::env;
use std::sync::Arc;

#[async_std::main]
async fn main() -> std::io::Result<()> {
    let bind_addr = env::var("IMAP_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:1143".to_string());
    let store = config::mail_store_from_env()?;
    println!("IMAPrev1 listening on {}...", bind_addr);

    let listener = TcpListener::bind(bind_addr.as_str()).await?;

    listener
        .incoming()
        .for_each_concurrent(None, |stream| {
            let store = Arc::clone(&store);

            async move {
                match stream {
                    Ok(stream) => {
                        let mut conn = connection::new(stream);
                        session::handle_connection(&mut conn, store.as_ref()).await;
                    }
                    Err(err) => eprintln!("Failed to accept connection: {}", err),
                }
            }
        })
        .await;

    Ok(())
}
