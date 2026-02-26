//! Probe and reflector helpers for integration tests.

use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use tracing::debug;

pub use crate::core::TaskHandle;
use crate::{
    core::{run_closure_in_namespace, spawn_closure_in_namespace_thread},
    ObservedAddr,
};

/// Spawns a UDP reflector in the named netns. Returns the task handle.
pub fn spawn_reflector_in(
    ns: &str,
    bind: SocketAddr,
) -> Result<(TaskHandle, thread::JoinHandle<Result<()>>)> {
    let ns = ns.to_string();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    let join = spawn_closure_in_namespace_thread(ns, move || {
        let sock = UdpSocket::bind(bind).context("reflector bind")?;
        let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
        let mut buf = [0u8; 512];
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            match sock.recv_from(&mut buf) {
                Ok((_, peer)) => {
                    let msg = format!("OBSERVED {}", peer);
                    let _ = sock.send_to(msg.as_bytes(), peer);
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    continue;
                }
                Err(_) => break,
            }
        }
        Ok(())
    });
    Ok((TaskHandle::new(stop_tx), join))
}

/// Sends a UDP probe from inside `ns` and returns the observed external address.
pub fn probe_in_ns(
    ns: &str,
    reflector: SocketAddr,
    timeout: Duration,
    bind_port: Option<u16>,
) -> Result<ObservedAddr> {
    let ns_name = ns.to_string();
    let ns_for_log = ns_name.clone();
    run_closure_in_namespace(&ns_name, move || {
        let unspecified = if reflector.is_ipv4() {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        };
        let bind_addr = SocketAddr::new(unspecified, bind_port.unwrap_or(0));
        let sock = UdpSocket::bind(bind_addr)?;
        sock.set_read_timeout(Some(timeout))?;
        let mut buf = [0u8; 512];
        for attempt in 1..=3 {
            sock.send_to(b"PROBE", reflector)?;
            match sock.recv_from(&mut buf) {
                Ok((n, _)) => {
                    let s = std::str::from_utf8(&buf[..n])?;
                    let addr_str = s
                        .strip_prefix("OBSERVED ")
                        .ok_or_else(|| anyhow!("unexpected reflector reply: {:?}", s))?;
                    return Ok(ObservedAddr {
                        observed: addr_str.parse()?,
                    });
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    debug!(
                        ns = %ns_for_log,
                        attempt,
                        "probe timeout waiting for reflector reply"
                    );
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(anyhow!("probe timed out after 3 attempts"))
    })
}

/// Returns the observed external address from a one-shot UDP probe in `ns`.
pub fn udp_roundtrip_in_ns(ns: &str, reflector: SocketAddr) -> Result<ObservedAddr> {
    probe_in_ns(ns, reflector, Duration::from_millis(500), None)
}

/// Returns UDP round-trip time from `ns` to `reflector`.
pub fn udp_rtt_in_ns(ns: &str, reflector: SocketAddr) -> Result<Duration> {
    run_closure_in_namespace(ns, move || {
        let bind = if reflector.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let sock = UdpSocket::bind(bind)?;
        sock.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut buf = [0u8; 256];
        let start = Instant::now();
        sock.send_to(b"PING", reflector)?;
        let _ = sock.recv_from(&mut buf)?;
        Ok(start.elapsed())
    })
}
