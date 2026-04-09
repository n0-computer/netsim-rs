//! Tests for internal IP address allocator correctness.
//!
//! Exercises the IX IP pool allocators for both v4 and v6, confirming that
//! every address returned is unique and the expected total count is exact.

use super::*;

/// The v4 IX low allocator returns 245 unique addresses before exhaustion.
///
/// Allocation starts at host .10 and runs to .254 of the IX /24, giving
/// 245 usable host addresses.
#[tokio::test(flavor = "current_thread")]
#[traced_test]
async fn ix_ip_v4_unique() -> Result<()> {
    let lab = Lab::new().await?;
    let mut ips = std::collections::HashSet::new();
    let mut inner = lab.inner.core.lock().unwrap();
    // next_ix_low starts at 10, so we can allocate 245 IPs (10..=254).
    let mut count = 0;
    while let Ok(ip) = inner.alloc_ix_ip_low() {
        assert!(ips.insert(ip), "duplicate IX IP {ip} at iteration {count}");
        count += 1;
    }
    assert_eq!(count, 245, "expected 245 unique IPs (hosts 10..=254)");
    Ok(())
}

/// The v6 IX low allocator returns 65519 unique addresses before exhaustion.
///
/// Allocation starts at index 0x10 (16) and runs to 0xFFFE (65534), giving
/// 65519 usable addresses.
#[tokio::test(flavor = "current_thread")]
#[traced_test]
async fn ix_ip_v6_unique() -> Result<()> {
    let lab = Lab::new().await?;
    let mut ips = std::collections::HashSet::new();
    let mut inner = lab.inner.core.lock().unwrap();
    // next_ix_low_v6 starts at 0x10 = 16.
    let mut count = 0u32;
    while let Ok(ip) = inner.alloc_ix_ip_v6_low() {
        assert!(
            ips.insert(ip),
            "duplicate IX v6 IP {ip} at iteration {count}"
        );
        count += 1;
    }
    // 16..=65534 = 65519 unique IPs
    assert_eq!(count, 65519, "expected 65519 unique v6 IPs");
    Ok(())
}

/// A /24 switch pool allows exactly 253 host allocations (hosts .2 through .254).
///
/// Host .1 is the gateway, .0 is network, .255 is broadcast. The allocator
/// starts at host index 2, so allocating 253 hosts should succeed, and the
/// 254th should fail.
#[tokio::test(flavor = "current_thread")]
#[traced_test]
async fn switch_pool_exhaustion() -> Result<()> {
    let lab = Lab::new().await?;
    let dc = lab.add_router("dc").build().await?;

    // Find the downlink switch for this router so we can allocate directly.
    let sw_id = {
        let inner = lab.inner.core.lock().unwrap();
        inner
            .router(dc.id())
            .expect("router should exist")
            .downlink
            .expect("router should have a downlink switch")
    };

    // Allocate 253 addresses (hosts .2 through .254).
    let mut ips = std::collections::HashSet::new();
    {
        let mut inner = lab.inner.core.lock().unwrap();
        for i in 0..253 {
            let ip = inner
                .alloc_from_switch(sw_id)
                .with_context(|| format!("allocation {i} should succeed"))?;
            assert!(ips.insert(ip), "duplicate IP {ip} at iteration {i}");
        }

        // The 254th allocation should fail — pool exhausted.
        let err = inner.alloc_from_switch(sw_id);
        assert!(err.is_err(), "254th allocation should fail");
    }

    assert_eq!(ips.len(), 253, "should have allocated exactly 253 unique IPs");
    Ok(())
}
