//! Enforceable outbound routes. A detected process is never evidence of a route.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::net::{TcpSocket, TcpStream};

#[derive(Clone, Debug)]
pub(crate) struct TunRoute {
    pub interface: String,
    pub source: IpAddr,
    #[cfg(target_os = "macos")]
    pub index: u32,
}

#[derive(Clone, Debug)]
pub(crate) enum OutboundRoute {
    Proxy(String),
    Tun(TunRoute),
    Direct,
}

impl OutboundRoute {
    pub fn cache_key(&self) -> String {
        match self {
            Self::Proxy(url) => url.clone(),
            Self::Tun(tun) => format!("tun:{}:{}", tun.interface, tun.source),
            Self::Direct => "direct/system".into(),
        }
    }
}

pub(crate) fn require_route(
    proxy: Option<String>,
    required: bool,
    tun: impl FnOnce() -> Result<TunRoute, String>,
) -> Result<OutboundRoute, String> {
    if let Some(proxy) = proxy {
        return Ok(OutboundRoute::Proxy(proxy));
    }
    if !required {
        return Ok(OutboundRoute::Direct);
    }
    tun().map(OutboundRoute::Tun).map_err(|e| {
        format!("Require proxy: no enforceable outbound proxy or active TUN route ({e}). Configure a global/account proxy or enable a local TUN route.")
    })
}

// Detection is cached briefly; every connection still binds its source/device.
pub(crate) fn detect_tun() -> Result<TunRoute, String> {
    static CACHE: once_cell::sync::Lazy<
        std::sync::Mutex<Option<(Instant, Result<TunRoute, String>)>>,
    > = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));
    let mut cache = CACHE.lock().map_err(|_| "TUN route cache unavailable")?;
    if let Some((at, result)) = &*cache {
        if at.elapsed() < Duration::from_secs(5) {
            return result.clone();
        }
    }
    let result = detect_tun_now();
    *cache = Some((Instant::now(), result.clone()));
    result
}

fn output(mut cmd: std::process::Command) -> Result<String, String> {
    use std::process::Stdio;
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            let result = child.wait_with_output().map_err(|e| e.to_string())?;
            if !result.status.success() {
                return Err("network route detection failed".into());
            }
            return String::from_utf8(result.stdout).map_err(|e| e.to_string());
        }
        if started.elapsed() >= Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("network route detection timed out".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
fn detect_tun_now() -> Result<TunRoute, String> {
    // Find-NetRoute consults the actual selected route and source, including
    // the split default routes local TUN clients install. Strong-host sends
    // ensure a socket bound to that source cannot leave an ordinary adapter.
    let script = r#"
        $ErrorActionPreference='Stop'
        $r=Find-NetRoute -RemoteIPAddress 1.1.1.1
        $s=$r | Where-Object { $_.IPAddress } | Select-Object -First 1
        if (!$s) { throw 'No route source' }
        $a=Get-NetAdapter -IncludeHidden | Where-Object { $_.ifIndex -eq $s.InterfaceIndex } | Select-Object -First 1
        if ($a.Status -ne 'Up' -or $a.InterfaceDescription -notmatch '(?i)wintun|\btun\b|tap-windows|wireguard|sing-box|mihomo|clash|v2ray') { throw 'Selected route is not a TUN adapter' }
        if (Get-NetIPInterface -AddressFamily IPv4 | Where-Object { $_.WeakHostSend -eq 'Enabled' }) { throw 'Weak-host routing prevents source enforcement' }
        @{interface=[string]$a.Name;source=[string]$s.IPAddress} | ConvertTo-Json -Compress
    "#;
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    let value: serde_json::Value =
        serde_json::from_str(&output(cmd)?).map_err(|e| e.to_string())?;
    let source = value["source"]
        .as_str()
        .ok_or("missing TUN source")?
        .parse::<IpAddr>()
        .map_err(|e| e.to_string())?;
    if source.is_loopback() || source.is_unspecified() || !source.is_ipv4() {
        return Err("invalid TUN source".into());
    }
    Ok(TunRoute {
        interface: value["interface"]
            .as_str()
            .ok_or("missing TUN interface")?
            .into(),
        source,
    })
}

#[cfg(target_os = "linux")]
fn detect_tun_now() -> Result<TunRoute, String> {
    let mut cmd = std::process::Command::new("ip");
    cmd.args(["-j", "route", "get", "1.1.1.1"]);
    let values: Vec<serde_json::Value> =
        serde_json::from_str(&output(cmd)?).map_err(|e| e.to_string())?;
    let route = values.first().ok_or("no selected route")?;
    let interface = route["dev"].as_str().ok_or("no route interface")?;
    if !std::path::Path::new(&format!("/sys/class/net/{interface}/tun_flags")).exists() {
        return Err("selected route is not a TUN device".into());
    }
    let source = route["prefsrc"]
        .as_str()
        .or_else(|| route["src"].as_str())
        .ok_or("no route source")?
        .parse::<IpAddr>()
        .map_err(|e| e.to_string())?;
    Ok(TunRoute {
        interface: interface.into(),
        source,
    })
}

#[cfg(target_os = "macos")]
fn detect_tun_now() -> Result<TunRoute, String> {
    let mut cmd = std::process::Command::new("route");
    cmd.args(["-n", "get", "1.1.1.1"]);
    let routes = output(cmd)?;
    let interface = routes
        .lines()
        .find_map(|l| l.trim().strip_prefix("interface:").map(str::trim))
        .ok_or("no route interface")?;
    if !interface.starts_with("utun") && !interface.starts_with("tun") {
        return Err("selected route is not a TUN interface".into());
    }
    let mut cmd = std::process::Command::new("ifconfig");
    cmd.arg(interface);
    let addresses = output(cmd)?;
    let source = addresses
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("inet ")
                .and_then(|s| s.split_whitespace().next())
        })
        .ok_or("no IPv4 TUN source")?
        .parse::<IpAddr>()
        .map_err(|e| e.to_string())?;
    extern "C" {
        fn if_nametoindex(name: *const std::os::raw::c_char) -> u32;
    }
    let name = std::ffi::CString::new(interface).map_err(|e| e.to_string())?;
    let index = unsafe { if_nametoindex(name.as_ptr()) };
    if index == 0 {
        return Err("TUN interface disappeared".into());
    }
    Ok(TunRoute {
        interface: interface.into(),
        source,
        index,
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn detect_tun_now() -> Result<TunRoute, String> {
    Err("TUN route detection unsupported on this platform".into())
}

pub(crate) async fn connect_direct(
    host: &str,
    port: u16,
    route: &OutboundRoute,
) -> Result<TcpStream, String> {
    let OutboundRoute::Tun(tun) = route else {
        return TcpStream::connect((host, port))
            .await
            .map_err(|e| e.to_string());
    };
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| e.to_string())?;
    let mut last = "destination has no address matching the TUN source".to_string();
    for target in addresses.filter(|a| a.is_ipv4() == tun.source.is_ipv4()) {
        let socket = if target.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }
        .map_err(|e| e.to_string())?;
        socket
            .bind(SocketAddr::new(tun.source, 0))
            .map_err(|e| format!("Require proxy: TUN source unavailable: {e}"))?;
        #[cfg(target_os = "linux")]
        socket2::SockRef::from(&socket)
            .bind_device(Some(tun.interface.as_bytes()))
            .map_err(|e| format!("Require proxy: cannot bind TUN interface: {e}"))?;
        #[cfg(target_os = "macos")]
        socket2::SockRef::from(&socket)
            .bind_device_by_index_v4(std::num::NonZeroU32::new(tun.index))
            .map_err(|e| format!("Require proxy: cannot bind TUN interface: {e}"))?;
        match socket.connect(target).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("Require proxy: TUN connection failed: {last}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_proxy_rejects_direct_and_accepts_proxy_or_tun() {
        assert!(require_route(None, true, || Err("ordinary Wi-Fi".into()))
            .unwrap_err()
            .contains("Require proxy"));
        assert!(matches!(
            require_route(None, false, || panic!("should not detect TUN")).unwrap(),
            OutboundRoute::Direct
        ));
        assert!(matches!(
            require_route(Some("socks5://127.0.0.1:1080".into()), true, || panic!(
                "explicit proxy takes precedence"
            ))
            .unwrap(),
            OutboundRoute::Proxy(_)
        ));
        assert!(matches!(
            require_route(None, true, || Ok(TunRoute {
                interface: "test_tun".into(),
                source: "192.0.2.1".parse().unwrap(),
                #[cfg(target_os = "macos")]
                index: 1
            }))
            .unwrap(),
            OutboundRoute::Tun(_)
        ));
    }

    #[tokio::test]
    async fn missing_tun_source_cannot_fall_back_to_direct() {
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let route = OutboundRoute::Tun(TunRoute {
            interface: "missing_tun".into(),
            source: "192.0.2.1".parse().unwrap(),
            #[cfg(target_os = "macos")]
            index: u32::MAX,
        });
        assert!(
            connect_direct("127.0.0.1", target.local_addr().unwrap().port(), &route)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(80), target.accept())
                .await
                .is_err()
        );
    }
}
