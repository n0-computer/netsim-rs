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
