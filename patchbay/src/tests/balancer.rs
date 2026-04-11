//! L4 load balancer tests.

use std::collections::HashMap;

use super::*;

/// Spawns a TCP server in a device namespace that responds with `ident` and closes.
async fn spawn_ident_server(device: &Device, port: u16, ident: &str) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let bind = SocketAddr::new(IpAddr::V4(device.ip().context("no ip")?), port);
    let ident = ident.to_string();
    device
        .spawn(move |_| async move {
            let listener = tokio::net::TcpListener::bind(bind).await?;
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    break;
                };
                let msg = ident.clone();
                tokio::spawn(async move {
                    let _ = stream.write_all(msg.as_bytes()).await;
                });
            }
            anyhow::Ok(())
        })?
        .await??;
    // Small delay for the listener to be ready.
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(())
}

/// Connects to `target` and reads the identity string.
async fn read_ident(target: SocketAddr) -> Result<String> {
    use tokio::io::AsyncReadExt;

    let timeout = Duration::from_millis(500);
    let mut stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(target))
        .await
        .context("tcp connect timeout")?
        .context("tcp connect")?;
    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(timeout, stream.read(&mut buf))
        .await
        .context("tcp read timeout")?
        .context("tcp read")?;
    Ok(String::from_utf8_lossy(&buf[..n]).to_string())
}

/// Round-robin balancer distributes connections across all backends.
#[tokio::test(flavor = "current_thread")]
#[traced_test]
async fn round_robin_distribution() -> Result<()> {
    check_caps()?;
    let lab = Lab::new().await?;

    let dc = lab.add_router("dc").build().await?;
    let web1 = lab
        .add_device("web1")
        .iface("eth0", dc.id())
        .build()
        .await?;
    let web2 = lab
        .add_device("web2")
        .iface("eth0", dc.id())
        .build()
        .await?;
    let web3 = lab
        .add_device("web3")
        .iface("eth0", dc.id())
        .build()
        .await?;
    let client = lab
        .add_device("client")
        .iface("eth0", dc.id())
        .build()
        .await?;

    // Start identity servers on each backend.
    spawn_ident_server(&web1, 8080, "web1").await?;
    spawn_ident_server(&web2, 8080, "web2").await?;
    spawn_ident_server(&web3, 8080, "web3").await?;

    // Use an IP in the same subnet but not in use as the VIP.
    let downstream_cidr = dc.downstream_cidr().context("no downstream cidr")?;
    let vip: Ipv4Addr = {
        let base = u32::from(downstream_cidr.addr());
        // Use .100 in the subnet as the VIP.
        Ipv4Addr::from(base + 100)
    };

    dc.add_balancer(
        BalancerConfig::new("web", vip, 80)
            .backend(web1.id(), 8080)
            .backend(web2.id(), 8080)
            .backend(web3.id(), 8080)
            .round_robin(),
    )
    .await?;

    // Make 9 connections from client. Round-robin should hit each backend 3 times.
    let target = SocketAddr::new(IpAddr::V4(vip), 80);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for _ in 0..9 {
        let ident = client
            .spawn(move |_| async move { read_ident(target).await })?
            .await??;
        *counts.entry(ident).or_default() += 1;
    }

    info!("round-robin distribution: {:?}", counts);
    assert_eq!(
        counts.len(),
        3,
        "expected 3 distinct backends, got {counts:?}"
    );
    for (name, count) in &counts {
        assert_eq!(
            *count, 3,
            "backend {name} got {count} connections, expected 3"
        );
    }

    Ok(())
}

/// Adding and removing backends updates the distribution.
#[tokio::test(flavor = "current_thread")]
#[traced_test]
async fn backend_add_remove() -> Result<()> {
    check_caps()?;
    let lab = Lab::new().await?;

    let dc = lab.add_router("dc").build().await?;
    let web1 = lab
        .add_device("web1")
        .iface("eth0", dc.id())
        .build()
        .await?;
    let web2 = lab
        .add_device("web2")
        .iface("eth0", dc.id())
        .build()
        .await?;
    let client = lab
        .add_device("client")
        .iface("eth0", dc.id())
        .build()
        .await?;

    spawn_ident_server(&web1, 8080, "web1").await?;
    spawn_ident_server(&web2, 8080, "web2").await?;

    let downstream_cidr = dc.downstream_cidr().context("no downstream cidr")?;
    let vip = Ipv4Addr::from(u32::from(downstream_cidr.addr()) + 100);
    let target = SocketAddr::new(IpAddr::V4(vip), 80);

    // Start with 2 backends.
    dc.add_balancer(
        BalancerConfig::new("web", vip, 80)
            .backend(web1.id(), 8080)
            .backend(web2.id(), 8080)
            .round_robin(),
    )
    .await?;

    // Verify both backends get traffic.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..4 {
        let ident = client
            .spawn(move |_| async move { read_ident(target).await })?
            .await??;
        seen.insert(ident);
    }
    assert_eq!(seen.len(), 2, "expected 2 backends before removal");

    // Remove web2.
    dc.remove_lb_backend("web", web2.id()).await?;
    dc.flush_lb_conntrack("web").await?;

    // All connections should now go to web1.
    for _ in 0..3 {
        let ident = client
            .spawn(move |_| async move { read_ident(target).await })?
            .await??;
        assert_eq!(ident, "web1", "expected only web1 after removing web2");
    }

    // Add web2 back.
    dc.add_lb_backend("web", web2.id(), 8080).await?;

    // Verify both backends get traffic again.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..4 {
        let ident = client
            .spawn(move |_| async move { read_ident(target).await })?
            .await??;
        seen.insert(ident);
    }
    assert_eq!(seen.len(), 2, "expected 2 backends after re-add");

    Ok(())
}

/// UDP balancer distributes packets across backends.
#[tokio::test(flavor = "current_thread")]
#[traced_test]
async fn udp_balancing() -> Result<()> {
    check_caps()?;
    let lab = Lab::new().await?;

    let dc = lab.add_router("dc").build().await?;
    let dns1 = lab
        .add_device("dns1")
        .iface("eth0", dc.id())
        .build()
        .await?;
    let dns2 = lab
        .add_device("dns2")
        .iface("eth0", dc.id())
        .build()
        .await?;
    let client = lab
        .add_device("client")
        .iface("eth0", dc.id())
        .build()
        .await?;

    let dns1_ip = dns1.ip().context("no ip")?;
    let dns2_ip = dns2.ip().context("no ip")?;

    // Start UDP echo servers that reply with their identity.
    let dns1_bind = SocketAddr::new(IpAddr::V4(dns1_ip), 5353);
    let dns2_bind = SocketAddr::new(IpAddr::V4(dns2_ip), 5353);
    dns1.spawn(move |_| async move {
        let sock = UdpSocket::bind(dns1_bind).await.unwrap();
        let mut buf = [0u8; 256];
        loop {
            let Ok((len, peer)) = sock.recv_from(&mut buf).await else {
                break;
            };
            let mut reply = b"dns1:".to_vec();
            reply.extend_from_slice(&buf[..len]);
            let _ = sock.send_to(&reply, peer).await;
        }
    })?;
    dns2.spawn(move |_| async move {
        let sock = UdpSocket::bind(dns2_bind).await.unwrap();
        let mut buf = [0u8; 256];
        loop {
            let Ok((len, peer)) = sock.recv_from(&mut buf).await else {
                break;
            };
            let mut reply = b"dns2:".to_vec();
            reply.extend_from_slice(&buf[..len]);
            let _ = sock.send_to(&reply, peer).await;
        }
    })?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let downstream_cidr = dc.downstream_cidr().context("no downstream cidr")?;
    let vip = Ipv4Addr::from(u32::from(downstream_cidr.addr()) + 100);

    dc.add_balancer(
        BalancerConfig::new("dns", vip, 53)
            .backend(dns1.id(), 5353)
            .backend(dns2.id(), 5353)
            .udp()
            .round_robin(),
    )
    .await?;

    let target = SocketAddr::new(IpAddr::V4(vip), 53);

    // Send UDP packets from different source ports to get different conntrack entries.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..4 {
        let ident: String = client
            .spawn(move |_| async move {
                let sock = UdpSocket::bind("0.0.0.0:0").await?;
                sock.send_to(b"query", target).await?;
                let mut buf = [0u8; 256];
                let len = tokio::time::timeout(Duration::from_millis(500), async {
                    let (len, _) = sock.recv_from(&mut buf).await?;
                    anyhow::Ok(len)
                })
                .await
                .context("udp recv timeout")??;
                let reply = String::from_utf8_lossy(&buf[..len]).to_string();
                let ident = reply.split(':').next().unwrap_or("").to_string();
                Ok(ident)
            })?
            .await??;
        seen.insert(ident);
    }

    info!("UDP distribution: {:?}", seen);
    assert_eq!(
        seen.len(),
        2,
        "expected both UDP backends to receive traffic"
    );

    Ok(())
}
