use std::io::Result;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task,
};

#[tokio::main]
async fn main() -> Result<()> {
    let server_sock = TcpListener::bind("0.0.0.0:10000").await?;
    loop {
        if let Ok((mut sock, _addr)) = server_sock.accept().await {
            task::spawn(async move {
                let mut buf = [0u8; 64];
                while let Ok(num_bytes) = sock.read(&mut buf).await {
                    if num_bytes == 0 {
                        break
                    }
                    sock.write_all(&buf[..num_bytes]).await?;
                }
                Result::Ok(())
            });
        }
    }
}
