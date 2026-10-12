//! `CONNECT` tunnels for the traffic Claude Code sends outside its API address.
//!
//! While Claude is routed, its settings carry `HTTPS_PROXY` pointing at the
//! proxy's own port, so its sign-in, its own login renewals and its telemetry
//! arrive here as `CONNECT host:port`. Each tunnel leaves through the current
//! Claude account's own proxy, or the global proxy when it has none, the route
//! its API requests take. When the account's proxy cannot be reached the
//! tunnel is refused rather than opened directly.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::hyper_client::ProxyStream;
use super::outbound_route::OutboundRoute;
use crate::app_config::AppType;
use crate::database::Database;

const MAX_HEAD: usize = 8192;

/// Whether the first bytes of a connection open a `CONNECT` request.
pub fn is_connect(peeked: &[u8]) -> bool {
    peeked.starts_with(b"CONNECT ")
}

pub async fn serve(mut client: TcpStream, db: Arc<Database>) {
    let (target, leftover) = match read_target(&mut client).await {
        Ok(v) => v,
        Err(e) => {
            log::debug!("[connect_tunnel] bad request: {e}");
            let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            return;
        }
    };
    let route = match current_claude_route(&db).await {
        Ok(route) => route,
        Err(e) => {
            log::warn!("[connect_tunnel] outbound route refused: {e}");
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            return;
        }
    };
    let label = crate::proxy::http_client::mask_url(&route.cache_key());

    let mut upstream =
        match tokio::time::timeout(std::time::Duration::from_secs(30), open(&target, &route))
            .await
            .unwrap_or_else(|_| Err("outbound connection timed out".into()))
        {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[connect_tunnel] {target} via {label} failed: {e}");
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                return;
            }
        };
    log::info!("[connect_tunnel] {target} via {label}");

    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .is_err()
    {
        return;
    }
    if !leftover.is_empty() && upstream.write_all(&leftover).await.is_err() {
        return;
    }
    let opened_route = route.cache_key();
    let transfer = tokio::io::copy_bidirectional(&mut client, &mut upstream);
    tokio::pin!(transfer);
    let mut check = tokio::time::interval(std::time::Duration::from_secs(3));
    check.tick().await;
    loop {
        tokio::select! {
            _ = &mut transfer => break,
            _ = check.tick() => {
                if !current_claude_route(&db).await.is_ok_and(|route| route.cache_key() == opened_route) {
                    log::info!("[connect_tunnel] {target}: outbound route changed or unavailable; closing tunnel");
                    break;
                }
            }
        }
    }
}

/// The route of the current Claude account: its own proxy when it has one,
/// otherwise the global proxy, otherwise none.
async fn current_claude_route(db: &Arc<Database>) -> Result<OutboundRoute, String> {
    let id = crate::settings::get_effective_current_provider(db, &AppType::Claude)
        .map_err(|e| e.to_string())?;
    let provider = match id {
        Some(id) => db
            .get_provider_by_id(&id, "claude")
            .map_err(|e| e.to_string())?,
        None => None,
    };
    let config = provider
        .as_ref()
        .and_then(|p| p.meta.as_ref())
        .and_then(|m| m.proxy_config.as_ref());
    if let Some(provider) = &provider {
        // This only schedules the background observation, including sign-in
        // traffic from clients that have not made a model request yet.
        let _ = super::account_pool::ensure_exit_allowed(
            db,
            provider,
            super::claude_pool::EXIT_TRACE_URL,
        )
        .await;
    }
    crate::proxy::http_client::resolve_route(config)
}

/// Reads the request head and returns `host:port` plus any bytes after it.
async fn read_target(client: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(i) = find_head_end(&buf) {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return Err("request head too long".into());
        }
        let n = client.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed before the request head ended".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]);
    let line = head.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    let (Some("CONNECT"), Some(target)) = (parts.next(), parts.next()) else {
        return Err(format!("not a CONNECT request: {line}"));
    };
    if !target.contains(':') {
        return Err(format!("target without a port: {target}"));
    }
    Ok((target.to_string(), buf[end..].to_vec()))
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

async fn open(target: &str, route: &OutboundRoute) -> Result<ProxyStream, String> {
    let (target_host, target_port) = target.rsplit_once(':').ok_or("target without a port")?;
    let target_host = target_host.trim_start_matches('[').trim_end_matches(']');
    let target_port = target_port.parse::<u16>().map_err(|_| "bad target port")?;
    let OutboundRoute::Proxy(route) = route else {
        return super::outbound_route::connect_direct(target_host, target_port, route)
            .await
            .map(ProxyStream::Tcp);
    };
    let url = url::Url::parse(route).map_err(|e| format!("bad proxy address: {e}"))?;
    let host = url.host_str().ok_or("proxy address without a host")?;
    let port = url
        .port_or_known_default()
        .ok_or("proxy address without a port")?;
    let decode = |s: &str| {
        percent_encoding::percent_decode_str(s)
            .decode_utf8_lossy()
            .into_owned()
    };
    let user = decode(url.username());
    let pass = url.password().map(decode).unwrap_or_default();
    if matches!(url.scheme(), "http" | "https") {
        return super::hyper_client::connect_via_proxy(route, target_host, target_port)
            .await
            .map_err(|e| e.to_string());
    }
    let mut stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("proxy unreachable: {e}"))?;
    match url.scheme() {
        "socks5" | "socks5h" => socks5_connect(&mut stream, target, &user, &pass).await?,
        other => return Err(format!("unsupported proxy type {other}")),
    }
    Ok(ProxyStream::Tcp(stream))
}

/// SOCKS5 (RFC 1928) with username/password auth (RFC 1929); the target name
/// is resolved by the proxy.
async fn socks5_connect(
    stream: &mut TcpStream,
    target: &str,
    user: &str,
    pass: &str,
) -> Result<(), String> {
    let (host, port) = target.rsplit_once(':').ok_or("target without a port")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port: u16 = port.parse().map_err(|_| "bad target port")?;
    let io = |e: std::io::Error| e.to_string();

    let with_auth = !user.is_empty();
    let methods: &[u8] = if with_auth { &[0x00, 0x02] } else { &[0x00] };
    let mut hello = vec![0x05, methods.len() as u8];
    hello.extend_from_slice(methods);
    stream.write_all(&hello).await.map_err(io)?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).await.map_err(io)?;
    match reply {
        [0x05, 0x00] => {}
        [0x05, 0x02] if with_auth => {
            if user.len() > 255 || pass.len() > 255 {
                return Err("SOCKS credentials too long".into());
            }
            let mut auth = vec![0x01, user.len() as u8];
            auth.extend_from_slice(user.as_bytes());
            auth.push(pass.len() as u8);
            auth.extend_from_slice(pass.as_bytes());
            stream.write_all(&auth).await.map_err(io)?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await.map_err(io)?;
            if status[1] != 0x00 {
                return Err("SOCKS proxy rejected the credentials".into());
            }
        }
        _ => {
            return Err(format!(
                "SOCKS proxy refused the methods offered: {reply:?}"
            ))
        }
    }

    if host.len() > 255 {
        return Err("target host too long".into());
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&req).await.map_err(io)?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(io)?;
    if head[1] != 0x00 {
        return Err(format!("SOCKS proxy could not connect (code {})", head[1]));
    }
    let addr_len = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(io)?;
            len[0] as usize
        }
        other => return Err(format!("SOCKS reply with unknown address type {other}")),
    };
    let mut rest = vec![0u8; addr_len + 2];
    stream.read_exact(&mut rest).await.map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_connect_request() {
        assert!(is_connect(b"CONNECT platform.claude.com:443 HTTP/1.1\r\n"));
        assert!(!is_connect(b"POST /v1/messages HTTP/1.1\r\n"));
    }

    #[test]
    fn finds_the_end_of_the_head() {
        assert_eq!(find_head_end(b"CONNECT a:1 HTTP/1.1\r\n\r\nxy"), Some(24));
        assert_eq!(find_head_end(b"CONNECT a:1 HTTP/1.1\r\n"), None);
    }

    #[tokio::test]
    async fn failed_connect_proxy_cannot_open_a_direct_tunnel() {
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target.local_addr().unwrap().to_string();
        let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        for scheme in ["http", "socks5"] {
            assert!(open(
                &target_addr,
                &OutboundRoute::Proxy(format!("{scheme}://{dead_addr}"))
            )
            .await
            .is_err());
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(80), target.accept())
                .await
                .is_err()
        );
    }
}
