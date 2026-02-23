//! Probe and reflector helpers for integration tests.
//!
//! These are thin wrappers around the free functions in the crate root that
//! give integration test authors a single import path.

pub use crate::{probe_in_ns, udp_roundtrip_in_ns, udp_rtt_in_ns};
pub use crate::core::TaskHandle;

use anyhow::Result;
use std::net::SocketAddr;
use std::thread;
use std::time::Duration;

use crate::core::spawn_closure_in_namespace_thread;
use std::io::ErrorKind;
use std::net::UdpSocket;
use anyhow::Context;

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
