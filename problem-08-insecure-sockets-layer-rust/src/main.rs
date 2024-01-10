use std::{pin::Pin, task::Poll};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum CipherOp {
    ReverseBits,
    Xor(u8),
    XorPos,
    Add(u8),
    AddPos,
}

struct CipherWriter<W: AsyncWrite + Unpin> {
    inner_writer: W,
    cipher_spec: Vec<CipherOp>,
    written: usize,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for CipherWriter<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let mut cipher_buf = Vec::from(buf);
        apply_cipher_spec(&mut cipher_buf, &self.cipher_spec, self.written);
        self.written += buf.len();
        Pin::new(&mut self.inner_writer).poll_write(cx, &cipher_buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.inner_writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.inner_writer).poll_shutdown(cx)
    }
}

impl<W: AsyncWrite + Unpin> CipherWriter<W> {
    fn new(inner_writer: W, cipher_spec: Vec<CipherOp>) -> Self {
        Self {
            inner_writer,
            cipher_spec,
            written: 0,
        }
    }
}

struct CipherReader<R: AsyncRead + Unpin> {
    inner_reader: R,
    cipher_spec: Vec<CipherOp>,
    read: usize,
}

impl<R: AsyncRead + Unpin> AsyncRead for CipherReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner_reader)
            .poll_read(cx, buf)
            .map_ok(|inner| {
                apply_cipher_spec_reverse(buf.filled_mut(), &self.cipher_spec, self.read);
                self.read += buf.filled().len();
                inner
            })
    }
}

impl<R: AsyncRead + Unpin> CipherReader<R> {
    fn new(inner_reader: R, cipher_spec: Vec<CipherOp>) -> Self {
        Self {
            inner_reader,
            cipher_spec,
            read: 0,
        }
    }
}

fn parse_cipher_spec(cipher: &[u8]) -> Vec<CipherOp> {
    let mut cipher_spec = vec![];
    let mut cipher_iter = cipher.into_iter();
    while let Some(n) = cipher_iter.next() {
        let op = match n {
            0x00 => break,
            0x01 => CipherOp::ReverseBits,
            0x02 => CipherOp::Xor(*cipher_iter.next().unwrap()),
            0x03 => CipherOp::XorPos,
            0x04 => CipherOp::Add(*cipher_iter.next().unwrap()),
            0x05 => CipherOp::AddPos,
            _ => break,
        };
        cipher_spec.push(op);
    }
    cipher_spec
}

fn is_noop_cipher_spec(cipher_spec: &[CipherOp]) -> bool {
    // The cipher cannot contain enough AddPos ops for it to wrap, since it is limited to 80 bytes
    if cipher_spec.into_iter().any(|op| op == &CipherOp::AddPos) {
        return false;
    }

    let before_buf: Vec<_> = (0..=255).collect();
    let mut buf = before_buf.clone();
    apply_cipher_spec(&mut buf, &cipher_spec, 0);

    before_buf == buf
}

fn apply_cipher_spec(buf: &mut [u8], cipher_spec: &[CipherOp], stream_pos: usize) {
    for (idx, val) in buf.iter_mut().enumerate() {
        for cipher_op in cipher_spec {
            match cipher_op {
                CipherOp::ReverseBits => *val = val.reverse_bits(),
                CipherOp::Xor(n) => *val = *val ^ *n,
                CipherOp::XorPos => *val = *val ^ (stream_pos + idx) as u8,
                CipherOp::Add(n) => *val = val.wrapping_add(*n),
                CipherOp::AddPos => *val = val.wrapping_add((stream_pos + idx) as u8),
            }
        }
    }
}

fn apply_cipher_spec_reverse(buf: &mut [u8], cipher_spec: &[CipherOp], stream_pos: usize) {
    for (idx, val) in buf.iter_mut().enumerate() {
        for cipher_op in cipher_spec.into_iter().rev() {
            match cipher_op {
                CipherOp::ReverseBits => *val = val.reverse_bits(),
                CipherOp::Xor(n) => *val = *val ^ *n,
                CipherOp::XorPos => *val = *val ^ (stream_pos + idx) as u8,
                CipherOp::Add(n) => *val = val.wrapping_sub(*n),
                CipherOp::AddPos => *val = val.wrapping_sub((stream_pos + idx) as u8),
            }
        }
    }
}

async fn handle_application_layer(
    reader: impl AsyncReadExt + Unpin,
    mut writer: impl AsyncWriteExt + Unpin,
) {
    let buf_reader = BufReader::new(reader);
    let mut lines_in = buf_reader.lines();

    while let Ok(Some(line)) = lines_in.next_line().await {
        if let Some((_, toy_string)) = line
            .split(",")
            .flat_map(|toy_string| {
                toy_string
                    .split("x")
                    .next()
                    .map(|amount_str| amount_str.parse::<i32>().ok())
                    .unwrap_or(None)
                    .map(|amount| (amount, toy_string))
            })
            .max_by_key(|(amount, _)| *amount)
        {
            let reply = format!("{}\n", toy_string);
            if let Err(_) = writer.write_all(reply.as_bytes()).await {
                return;
            }
        }
    }
}

async fn handle_socket(socket: TcpStream) {
    let (reader, writer) = socket.into_split();

    let mut cipher_bytes = Vec::with_capacity(80);
    let mut buf_reader = BufReader::new(reader);
    if let Err(_) = buf_reader.read_until(0, &mut cipher_bytes).await {
        return;
    };

    let cipher_spec = parse_cipher_spec(&cipher_bytes);

    if is_noop_cipher_spec(&cipher_spec) {
        return;
    }

    let cipher_reader = CipherReader::new(buf_reader, cipher_spec.clone());
    let cipher_writer = CipherWriter::new(writer, cipher_spec);

    handle_application_layer(cipher_reader, cipher_writer).await;
}

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:10000")
        .await
        .expect("socket bind");
    loop {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(handle_socket(socket));
        }
    }
}
