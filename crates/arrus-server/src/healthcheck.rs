//! `arrus-server healthcheck`: a std-only HTTP probe of `/healthz` for container
//! readiness checks (the runtime image has no curl).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

/// GET `http://127.0.0.1:<port>/healthz`; true on `200` with body `ok`.
pub fn probe(port: u16) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| format!("connect {addr}: {e}"))?;
    s.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    s.write_all(format!("GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    let mut buf = Vec::new();
    s.take(64 * 1024).read_to_end(&mut buf).map_err(|e| format!("read: {e}"))?;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("malformed response")?;
    let status_line = head.lines().next().unwrap_or("");
    if status_line.split_whitespace().nth(1) != Some("200") {
        return Err(format!("unexpected status: {status_line}"));
    }
    if body.trim() != "ok" {
        return Err(format!("unexpected body: {:?}", body.chars().take(100).collect::<String>()));
    }
    Ok(())
}
