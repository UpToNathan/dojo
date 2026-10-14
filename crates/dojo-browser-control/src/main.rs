use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use dojo_browser_control::{EXTENSION_ORIGIN, Event, MAX_FRAME, Request, unix};
use sha1::{Digest, Sha1};
const MAX_REPLY: usize = 32 * 1024 * 1024;
use std::{
    io::{self, BufRead, Read, Write},
    net::{TcpListener, TcpStream},
    os::fd::OwnedFd,
    os::unix::net::UnixDatagram,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Clone)]
struct Output(Arc<Mutex<io::Stdout>>);
impl Output {
    fn emit(&self, event: &Event) {
        if let Ok(mut stdout) = self.0.lock() {
            if serde_json::to_writer(&mut *stdout, event).is_ok() {
                let _ = stdout.write_all(b"\n");
                let _ = stdout.flush();
            }
        }
    }
}
struct ReadSocket(TcpStream);
impl ReadSocket {
    fn exact(&mut self, mut bytes: &mut [u8]) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        while !bytes.is_empty() {
            match self.0.read(bytes) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "extension disconnected",
                    ));
                }
                Ok(n) => bytes = &mut bytes[n..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let mut readiness = libc::pollfd {
                        fd: self.0.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    if unsafe { libc::poll(&mut readiness, 1, -1) } < 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::Interrupted {
                            return Err(error);
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    fn frame(&mut self) -> Result<(bool, u8, Vec<u8>)> {
        let mut h = [0u8; 2];
        self.exact(&mut h)?;
        ensure!(h[0] & 0x70 == 0, "compression unsupported");
        ensure!(h[1] & 0x80 != 0, "client frame must be masked");
        let opcode = h[0] & 15;
        ensure!(matches!(opcode, 0 | 1 | 8 | 9 | 10), "unsupported opcode");
        let fin = h[0] & 0x80 != 0;
        ensure!(opcode < 8 || fin, "fragmented control frame");
        let mut len = (h[1] & 127) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            self.exact(&mut b)?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            self.exact(&mut b)?;
            len = u64::from_be_bytes(b);
        }
        ensure!(
            len <= MAX_REPLY as u64 && (opcode < 8 || len <= 125),
            "oversized frame"
        );
        let mut mask = [0u8; 4];
        self.exact(&mut mask)?;
        let mut payload = vec![0u8; len as usize];
        self.exact(&mut payload)?;
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
        Ok((fin, opcode, payload))
    }
}
fn handshake(mut stream: TcpStream) -> Result<TcpStream> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        ensure!(request.len() < 8192, "oversized handshake");
        let mut b = [0];
        stream.read_exact(&mut b)?;
        request.push(b[0]);
    }
    let text = std::str::from_utf8(&request)?;
    let mut lines = text.split("\r\n");
    ensure!(
        lines.next() == Some("GET /extension HTTP/1.1"),
        "wrong endpoint"
    );
    let mut headers = std::collections::BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (k, v) = line.split_once(':').context("bad header")?;
        ensure!(
            headers.insert(k.to_ascii_lowercase(), v.trim()).is_none(),
            "duplicate header"
        );
    }
    ensure!(
        headers.get("origin") == Some(&EXTENSION_ORIGIN),
        "extension origin refused"
    );
    ensure!(
        headers
            .get("upgrade")
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
            && headers.get("sec-websocket-version") == Some(&"13"),
        "invalid websocket upgrade"
    );
    ensure!(
        headers.get("connection").is_some_and(|v| v
            .split(',')
            .any(|s| s.trim().eq_ignore_ascii_case("upgrade"))),
        "invalid connection header"
    );
    let key = headers
        .get("sec-websocket-key")
        .context("missing websocket key")?;
    ensure!(STANDARD.decode(key)?.len() == 16, "invalid websocket key");
    let accept = STANDARD.encode(Sha1::digest(format!(
        "{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
    )));
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    stream.flush()?;
    stream.set_read_timeout(None)?;
    Ok(stream)
}
fn main() -> Result<()> {
    ensure!(
        std::env::args().skip(1).collect::<Vec<_>>() == ["--stdio"],
        "usage: dojo-browser-control-broker --stdio"
    );
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR missing")?);
    let native = std::env::var_os("DOJO_BROWSER_CONTROL_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime.join("dojo-browser-admission.sock"));
    let path = runtime.join(format!("dojo-browser-broker-{}.sock", std::process::id()));
    let socket = Arc::new(UnixDatagram::bind(&path)?);
    dojo_browser_control::configure_socket_buffers(&*socket)?;
    socket
        .connect(&native)
        .context("connect native browser admission")?;
    let output = Output(Arc::new(Mutex::new(io::stdout())));
    let registered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let listener = TcpListener::bind("127.0.0.1:19988").context("bind extension endpoint")?;
    let receive = socket.clone();
    let out = output.clone();
    let connected = registered.clone();
    thread::spawn(move || {
        let mut bytes = vec![0u8; MAX_FRAME];
        while let Ok(n) = receive.recv(&mut bytes) {
            match serde_json::from_slice::<Event>(&bytes[..n]) {
                Ok(event) => {
                    if matches!(&event,Event::Result{id:0,status,..} if status=="registered") {
                        connected.store(true, std::sync::atomic::Ordering::Release);
                        out.emit(&Event::Connected);
                    } else {
                        out.emit(&event);
                    }
                }
                Err(e) => eprintln!("native reply: {e}"),
            }
        }
    });
    let ingress = socket.clone();
    let endpoint = native.clone();
    let out = output.clone();
    let connected = registered.clone();
    thread::spawn(move || {
        for connection in listener.incoming() {
            let result = (|| -> Result<()> {
                let stream = handshake(connection?)?;
                ensure!(
                    !connected.load(std::sync::atomic::Ordering::Acquire),
                    "extension already connected"
                );
                let reader = ReadSocket(stream.try_clone()?);
                let fd: OwnedFd = stream.into();
                unix::send(&ingress, &endpoint, &Request::Register { id: 0 }, Some(&fd))?;
                drop(fd);
                let mut reader = reader;
                let mut fragmented: Option<Vec<u8>> = None;
                let mut fragments = 0usize;
                loop {
                    let (fin, op, payload) = reader.frame()?;
                    match op {
                        0 | 1 => {
                            ensure!(
                                (op == 1) == fragmented.is_none(),
                                "invalid continuation frame"
                            );
                            let mut payload = if let Some(mut pending) = fragmented.take() {
                                ensure!(
                                    pending.len() + payload.len() <= MAX_REPLY,
                                    "oversized fragmented reply"
                                );
                                pending.extend_from_slice(&payload);
                                pending
                            } else {
                                payload
                            };
                            fragments += 1;
                            ensure!(fragments <= 4096, "too many reply fragments");
                            if !fin {
                                fragmented = Some(std::mem::take(&mut payload));
                                continue;
                            }
                            fragments = 0;
                            let message: serde_json::Value = serde_json::from_slice(&payload)?;
                            if let Some(extension_id) =
                                message.get("id").and_then(serde_json::Value::as_u64)
                            {
                                unix::send(
                                    &ingress,
                                    &endpoint,
                                    &Request::Settle {
                                        extension_id,
                                        completed: message.get("error").is_none(),
                                    },
                                    None,
                                )?;
                            }
                            out.emit(&Event::Extension { message });
                        }
                        9 => unix::send(&ingress, &endpoint, &Request::Pong { payload }, None)?,
                        8 => break,
                        10 => {}
                        _ => unreachable!(),
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("extension connection: {e:#}");
            }
            let _ = unix::send(&ingress, &endpoint, &Request::Disconnect, None);
            connected.store(false, std::sync::atomic::Ordering::Release);
            out.emit(&Event::Disconnected);
        }
    });
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        let n = input
            .by_ref()
            .take((MAX_FRAME + 1) as u64)
            .read_line(&mut line)?;
        if n == 0 {
            break;
        }
        ensure!(
            n <= MAX_FRAME && line.ends_with('\n'),
            "oversized or incomplete stdio line"
        );
        let request: Request = serde_json::from_str(&line)?;
        ensure!(
            matches!(
                request,
                Request::Bind { .. } | Request::Send { .. } | Request::Retire { .. }
            ),
            "internal native operations are not stdio commands"
        );
        if let Err(e) = unix::send(&socket, &native, &request, None) {
            match request {
                Request::Bind { id, .. }
                | Request::Send { id, .. }
                | Request::Retire { id, .. } => {
                    output.emit(&Event::result(id, "refused", Some(&e.to_string())))
                }
                _ => unreachable!(),
            }
        }
    }
    let _ = unix::send(&socket, &native, &Request::Disconnect, None);
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extension_upgrade_preserves_socket_for_native_writer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || handshake(listener.accept().unwrap().0).unwrap());
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(format!("GET /extension HTTP/1.1\r\nHost: localhost\r\nOrigin: {EXTENSION_ORIGIN}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").as_bytes()).unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let mut b = [0];
            client.read_exact(&mut b).unwrap();
            header.push(b[0]);
        }
        assert!(
            String::from_utf8(header)
                .unwrap()
                .contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
        );
        let mut receive = ReadSocket(server.join().unwrap());
        receive.0.set_nonblocking(true).unwrap();
        let sending = thread::spawn(move || {
            thread::sleep(Duration::from_millis(5));
            client
                .write_all(&[0x81, 0x82, 1, 2, 3, 4, b'h' ^ 1, b'i' ^ 2])
                .unwrap();
            client
        });
        let (fin, opcode, payload) = receive.frame().unwrap();
        assert!(fin);
        assert_eq!(opcode, 1);
        assert_eq!(payload, b"hi");
        let mut client = sending.join().unwrap();
        client.set_nonblocking(true).unwrap();
        let mut b = [0];
        assert_eq!(
            client.read(&mut b).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        ); // reader never writes
    }
}
