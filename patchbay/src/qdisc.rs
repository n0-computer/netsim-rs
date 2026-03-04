use anyhow::{bail, Context, Result};
use tokio::process::Command;

/// Max retries for transient EAGAIN (os error 11) when spawning `tc` commands.
const SPAWN_RETRIES: u32 = 3;

/// Parameters for `tc netem` impairment.
///
/// All fields default to zero (no impairment). Set only the fields you need.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LinkLimits {
    /// Rate limit in kbit/s (0 = unlimited).
    pub rate_kbit: u32,
    /// Packet loss percentage (0.0–100.0).
    pub loss_pct: f32,
    /// One-way latency in milliseconds.
    pub latency_ms: u32,
    /// Jitter in milliseconds (uniform ±jitter around latency).
    pub jitter_ms: u32,
    /// Packet reordering percentage (0.0–100.0).
    pub reorder_pct: f32,
    /// Packet duplication percentage (0.0–100.0).
    pub duplicate_pct: f32,
    /// Bit-error corruption percentage (0.0–100.0).
    pub corrupt_pct: f32,
}

/// Applies netem impairment on `ifname`. Caller must already be in the target ns.
pub(crate) async fn apply_impair(ifname: &str, limits: LinkLimits) -> Result<()> {
    remove_qdisc(ifname).await;
    let qdisc = Qdisc::new(ifname);
    qdisc.add_netem_root(limits).await?;
    if limits.rate_kbit > 0 {
        qdisc.add_tbf(limits.rate_kbit).await?;
    }
    Ok(())
}

pub(crate) async fn remove_qdisc(ifname: &str) {
    let qdisc = Qdisc::new(ifname);
    qdisc.clear_root().await;
}

struct Qdisc<'a> {
    ifname: &'a str,
}

impl<'a> Qdisc<'a> {
    fn new(ifname: &'a str) -> Self {
        Self { ifname }
    }

    async fn clear_root(&self) {
        let mut cmd = Command::new("tc");
        cmd.args(["qdisc", "del", "dev", self.ifname, "root"]);
        let _ = ensure_success(cmd, "tc qdisc del root").await;
    }

    async fn add_netem_root(&self, limits: LinkLimits) -> Result<()> {
        let mut args: Vec<String> = vec![
            "qdisc",
            "add",
            "dev",
            self.ifname,
            "root",
            "handle",
            "1:",
            "netem",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        if limits.latency_ms > 0 || limits.jitter_ms > 0 {
            args.push("delay".into());
            args.push(format!("{}ms", limits.latency_ms));
            if limits.jitter_ms > 0 {
                args.push(format!("{}ms", limits.jitter_ms));
            }
        }
        if limits.loss_pct > 0.0 {
            args.push("loss".into());
            args.push(format!("{:.3}%", limits.loss_pct));
        }
        if limits.reorder_pct > 0.0 {
            args.push("reorder".into());
            args.push(format!("{:.3}%", limits.reorder_pct));
        }
        if limits.duplicate_pct > 0.0 {
            args.push("duplicate".into());
            args.push(format!("{:.3}%", limits.duplicate_pct));
        }
        if limits.corrupt_pct > 0.0 {
            args.push("corrupt".into());
            args.push(format!("{:.3}%", limits.corrupt_pct));
        }

        let mut cmd = Command::new("tc");
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        cmd.args(&arg_refs);
        ensure_success(cmd, "tc qdisc netem add").await?;
        Ok(())
    }

    async fn add_tbf(&self, rate_kbit: u32) -> Result<()> {
        let mut cmd = Command::new("tc");
        cmd.args([
            "qdisc",
            "add",
            "dev",
            self.ifname,
            "parent",
            "1:1",
            "handle",
            "10:",
            "tbf",
            "rate",
            &format!("{}kbit", rate_kbit),
            "burst",
            "32kbit",
            "latency",
            "400ms",
        ]);
        ensure_success(cmd, "tc qdisc tbf add").await?;
        Ok(())
    }
}

async fn ensure_success(mut cmd: Command, context: &str) -> Result<()> {
    // Retry on transient EAGAIN (os error 11) which can happen on
    // resource-constrained CI runners when many namespaces are being
    // created/torn down in quick succession.
    cmd.stderr(std::process::Stdio::piped());
    for attempt in 0..=SPAWN_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(50 * attempt as u64)).await;
        }
        match cmd.output().await {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                bail!("{context} failed: {stderr}");
            }
            Err(e) if e.raw_os_error() == Some(11) && attempt < SPAWN_RETRIES => {
                tracing::debug!(%context, attempt, "EAGAIN, retrying");
            }
            Err(e) => {
                return Err(e).with_context(|| format!("{context}: spawn"));
            }
        }
    }
    bail!("{context}: spawn: EAGAIN after {SPAWN_RETRIES} retries");
}
