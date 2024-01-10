use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    sync::{oneshot, Mutex},
};

const PORT: u16 = 10000;

const MAX_NAME_LENGTH: usize = 32;
const MAX_CONNECTIONS: usize = 64;

#[derive(Debug, Default)]
struct ServerContext {
    joined_clients: HashMap<String, Client>,
    connections: usize,
}

impl ServerContext {
    fn new() -> Self {
        Self::default()
    }

    fn get_name_string(&self) -> String {
        self.joined_clients
            .keys()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(", ")
    }

    async fn add_client(&mut self, mut client: Client) {
        let current_users_string = format!("* The room contains: {}\n", self.get_name_string());
        client.send_message(&current_users_string).await;

        let join_message = format!("* {} has entered the room\n", client.name);
        self.broadcast(&join_message, None).await;

        self.joined_clients.insert(client.name.clone(), client);
    }

    async fn handle_client_message(&mut self, name: &str, message: &str) {
        let message = format!("[{}] {}\n", name, message);
        self.broadcast(&message, Some(name)).await;
    }

    async fn broadcast(&mut self, message: &str, exclude_name: Option<&str>) {
        for joined_client in self.joined_clients.values_mut() {
            if let Some(name) = exclude_name {
                if name == joined_client.name {
                    continue;
                }
            }

            joined_client.send_message(message).await;
        }
    }

    async fn remove_client(&mut self, name: &str) {
        if let Some(mut client) = self.joined_clients.remove(name) {
            let _ = client.writer.shutdown().await;
            let leave_message = format!("* {} has left the room\n", client.name);
            self.broadcast(&leave_message, None).await;
        }
    }

    fn is_valid_name(&self, name: &str) -> bool {
        !name.is_empty()
            && name.len() <= MAX_NAME_LENGTH
            && name.chars().all(|c| c.is_ascii_alphanumeric())
            && !self.joined_clients.contains_key(name)
    }
}

#[derive(Debug)]
struct Client {
    name: String,
    writer: OwnedWriteHalf,
    disconnect_sender: Option<oneshot::Sender<()>>,
}

impl Client {
    fn new(name: String, writer: OwnedWriteHalf, disconnect_sender: oneshot::Sender<()>) -> Self {
        Self {
            name,
            writer,
            disconnect_sender: Some(disconnect_sender),
        }
    }

    async fn send_message(&mut self, message: &str) {
        if self.writer.write_all(message.as_bytes()).await.is_err() {
            if let Some(disconnect_sender) = self.disconnect_sender.take() {
                let _ = disconnect_sender.send(());
            }
        }
    }
}

async fn handle_socket(
    ctx: Arc<Mutex<ServerContext>>,
    socket: TcpStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);

    writer
        .write_all(b"Welcome to budget chat! What shall I call you?\n")
        .await?;

    let mut name_line = String::new();
    let bytes_read = reader.read_line(&mut name_line).await?;
    if bytes_read == 0 {
        return Ok(());
    }
    let name = name_line.trim_end_matches('\n');

    let (disconnect_sender, disconnect_receiver) = oneshot::channel();

    {
        let mut ctx = ctx.lock().await;

        if !ctx.is_valid_name(&name) {
            drop(ctx);
            let _ = writer.write_all(b"Invalid or duplicate name").await;
            return Err("Invalid or duplicate name".into());
        };

        let client = Client::new(name.to_owned(), writer, disconnect_sender);
        ctx.add_client(client).await;
    }

    let _ = client_loop(ctx.clone(), &name, reader, disconnect_receiver).await;

    ctx.lock().await.remove_client(&name).await;

    Ok(())
}

async fn client_loop(
    ctx: Arc<Mutex<ServerContext>>,
    name: &str,
    mut reader: BufReader<OwnedReadHalf>,
    mut disconnect_receiver: oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut in_bytes = Vec::new();
    loop {
        in_bytes.clear();

        let bytes_read = tokio::select! {
            _ = &mut disconnect_receiver => {
                return Ok(());
            },
            bytes_read_result = reader.read_until(b'\n', &mut in_bytes) => {
                bytes_read_result?
            }
        };

        if bytes_read == 0 {
            return Ok(());
        }

        if in_bytes.last() == Some(&b'\n') {
            in_bytes.pop();
        }

        let in_string = String::from_utf8_lossy(&in_bytes);
        ctx.lock()
            .await
            .handle_client_message(name, &in_string)
            .await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), PORT);
    let listener = TcpListener::bind(addr).await?;

    let ctx = Arc::new(Mutex::new(ServerContext::new()));

    loop {
        let (socket, _) = listener.accept().await?;

        {
            let mut ctx = ctx.lock().await;
            if ctx.connections >= MAX_CONNECTIONS {
                continue;
            }

            ctx.connections += 1;
        }

        let task_ctx = ctx.clone();
        tokio::spawn(async move {
            let _ = handle_socket(task_ctx.clone(), socket).await;
            task_ctx.lock().await.connections -= 1;
        });
    }
}
