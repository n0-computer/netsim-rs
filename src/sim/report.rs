use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Parsed result from one iroh-transfer run.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransferResult {
    pub id: String,
    pub provider: String,
    pub fetcher: String,
    /// Bytes transferred.
    pub size_bytes: Option<u64>,
    /// Transfer duration in seconds.
    pub elapsed_s: Option<f64>,
    /// Throughput in Mbit/s.
    pub mbps: Option<f64>,
    /// Was the final connection direct (not relay)?
    pub final_conn_direct: Option<bool>,
    /// Did the connection ever upgrade to direct?
    pub conn_upgrade: Option<bool>,
    /// Total number of ConnectionTypeChanged events observed.
    pub conn_events: usize,
}

impl TransferResult {
    /// Parse a fetcher NDJSON log file and fill in transfer stats.
    pub fn parse_fetcher_log(&mut self, log_path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(log_path)
            .with_context(|| format!("read fetcher log {}", log_path.display()))?;
        let mut conn_events = 0usize;
        let mut ever_direct = false;
        let mut last_direct: Option<bool> = None;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match v.get("kind").and_then(|k| k.as_str()) {
                Some("DownloadComplete") => {
                    if let Some(size) = v.get("size").and_then(|s| s.as_u64()) {
                        self.size_bytes = Some(size);
                    }
                    if let Some(dur_us) = v.get("duration").and_then(|d| d.as_u64()) {
                        let elapsed = dur_us as f64 / 1_000_000.0;
                        self.elapsed_s = Some(elapsed);
                        if let Some(size) = self.size_bytes {
                            self.mbps = Some(size as f64 * 8.0 / (elapsed * 1_000_000.0));
                        }
                    }
                }
                Some("ConnectionTypeChanged") => {
                    if v.get("status").and_then(|s| s.as_str()) == Some("Selected") {
                        conn_events += 1;
                        let is_direct = v
                            .get("addr")
                            .and_then(|a| a.as_str())
                            .map(|a| a.contains("Ip("))
                            .unwrap_or(false);
                        if is_direct {
                            ever_direct = true;
                        }
                        last_direct = Some(is_direct);
                    }
                }
                _ => {}
            }
        }

        self.conn_events = conn_events;
        self.final_conn_direct = last_direct;
        self.conn_upgrade = Some(ever_direct);
        Ok(())
    }
}

/// Write results to `<work_dir>/results.json` and `<work_dir>/results.md`.
pub async fn write_results(
    work_dir: &Path,
    sim_name: &str,
    results: &[TransferResult],
) -> Result<()> {
    if results.is_empty() {
        return Ok(());
    }

    let json = serde_json::to_string_pretty(&serde_json::json!({
        "sim": sim_name,
        "transfers": results,
    }))
    .context("serialize results")?;
    tokio::fs::write(work_dir.join("results.json"), json)
        .await
        .context("write results.json")?;

    let mut md = String::new();
    md.push_str("| sim | id | provider | fetcher | size_bytes | elapsed_s | mbps | final_conn_direct | conn_upgrade | conn_events |\n");
    md.push_str("| --- | -- | -------- | ------- | ---------- | --------- | ---- | ----------------- | ------------ | ----------- |\n");
    for r in results {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            sim_name,
            r.id,
            r.provider,
            r.fetcher,
            r.size_bytes.map(|v| v.to_string()).unwrap_or_default(),
            r.elapsed_s.map(|v| format!("{:.3}", v)).unwrap_or_default(),
            r.mbps.map(|v| format!("{:.1}", v)).unwrap_or_default(),
            r.final_conn_direct
                .map(|v| v.to_string())
                .unwrap_or_default(),
            r.conn_upgrade.map(|v| v.to_string()).unwrap_or_default(),
            r.conn_events,
        ));
    }
    tokio::fs::write(work_dir.join("results.md"), md)
        .await
        .context("write results.md")?;

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunResults {
    run: String,
    sim: String,
    transfers: Vec<TransferResult>,
}

/// Scan run directories under `work_root` and emit combined reports.
///
/// If `run_names` is non-empty, only those run directories are included.
pub async fn write_combined_results_for_runs(work_root: &Path, run_names: &[String]) -> Result<()> {
    let include: Option<HashSet<&str>> = if run_names.is_empty() {
        None
    } else {
        Some(run_names.iter().map(String::as_str).collect())
    };
    let mut runs = Vec::new();
    for ent in
        std::fs::read_dir(work_root).with_context(|| format!("read {}", work_root.display()))?
    {
        let ent = ent?;
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name == "latest" {
            continue;
        }
        if let Some(filter) = &include {
            if !filter.contains(name) {
                continue;
            }
        }
        let results_json = path.join("results.json");
        if !results_json.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&results_json)
            .with_context(|| format!("read {}", results_json.display()))?;
        let v: serde_json::Value = serde_json::from_str(&text).context("parse run results json")?;
        let sim = v
            .get("sim")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let transfers: Vec<TransferResult> = serde_json::from_value(
            v.get("transfers")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::Array(vec![])),
        )
        .context("parse transfers array")?;
        runs.push(RunResults {
            run: name.to_string(),
            sim,
            transfers,
        });
    }

    runs.sort_by(|a, b| a.run.cmp(&b.run));

    let all_json = serde_json::to_string_pretty(&serde_json::json!({
        "runs": runs,
    }))
    .context("serialize combined results")?;
    tokio::fs::write(work_root.join("combined-results.json"), all_json)
        .await
        .context("write combined-results.json")?;

    let mut by_sim: BTreeMap<String, Vec<&TransferResult>> = BTreeMap::new();
    for run in &runs {
        for t in &run.transfers {
            by_sim.entry(run.sim.clone()).or_default().push(t);
        }
    }

    let mut md = String::new();
    md.push_str("| sim | transfers | avg_mbps | direct_final_pct |\n");
    md.push_str("| --- | --------- | -------- | ---------------- |\n");
    for (sim, transfers) in &by_sim {
        let mut mbps_sum = 0.0f64;
        let mut mbps_count = 0usize;
        let mut direct_total = 0usize;
        let mut direct_yes = 0usize;
        for t in transfers {
            if let Some(v) = t.mbps {
                mbps_sum += v;
                mbps_count += 1;
            }
            if let Some(v) = t.final_conn_direct {
                direct_total += 1;
                if v {
                    direct_yes += 1;
                }
            }
        }
        let avg_mbps = if mbps_count > 0 {
            format!("{:.1}", mbps_sum / mbps_count as f64)
        } else {
            String::new()
        };
        let direct_pct = if direct_total > 0 {
            format!(
                "{:.0}%",
                100.0 * (direct_yes as f64) / (direct_total as f64)
            )
        } else {
            String::new()
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            sim,
            transfers.len(),
            avg_mbps,
            direct_pct
        ));
    }
    md.push('\n');
    md.push_str("| run | sim | id | provider | fetcher | size_bytes | elapsed_s | mbps | final_conn_direct | conn_upgrade | conn_events |\n");
    md.push_str("| --- | --- | -- | -------- | ------- | ---------- | --------- | ---- | ----------------- | ------------ | ----------- |\n");
    for run in &runs {
        for r in &run.transfers {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                run.run,
                run.sim,
                r.id,
                r.provider,
                r.fetcher,
                r.size_bytes.map(|v| v.to_string()).unwrap_or_default(),
                r.elapsed_s.map(|v| format!("{:.3}", v)).unwrap_or_default(),
                r.mbps.map(|v| format!("{:.1}", v)).unwrap_or_default(),
                r.final_conn_direct
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                r.conn_upgrade.map(|v| v.to_string()).unwrap_or_default(),
                r.conn_events,
            ));
        }
    }
    tokio::fs::write(work_root.join("combined-results.md"), md)
        .await
        .context("write combined-results.md")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("netsim-{prefix}-{ts}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_fetcher_log_extracts_transfer_and_conn_fields() {
        let dir = temp_dir("report-parse");
        let log = dir.join("fetcher.ndjson");
        let data = r#"{"kind":"ConnectionTypeChanged","status":"Selected","addr":"Relay(http://r)"}
{"kind":"ConnectionTypeChanged","status":"Selected","addr":"Ip(1.2.3.4:9999)"}
{"kind":"DownloadComplete","size":1000,"duration":2000000}
"#;
        std::fs::write(&log, data).unwrap();

        let mut r = TransferResult::default();
        r.parse_fetcher_log(&log).unwrap();

        assert_eq!(r.size_bytes, Some(1000));
        assert_eq!(r.elapsed_s, Some(2.0));
        assert_eq!(r.mbps, Some(0.004));
        assert_eq!(r.final_conn_direct, Some(true));
        assert_eq!(r.conn_upgrade, Some(true));
        assert_eq!(r.conn_events, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_results_writes_json_and_markdown() {
        let dir = temp_dir("report-write");
        let results = vec![TransferResult {
            id: "xfer".to_string(),
            provider: "p".to_string(),
            fetcher: "f".to_string(),
            size_bytes: Some(42),
            elapsed_s: Some(1.5),
            mbps: Some(0.2),
            final_conn_direct: Some(false),
            conn_upgrade: Some(false),
            conn_events: 1,
        }];
        write_results(&dir, "sim-a", &results).await.unwrap();

        let json = std::fs::read_to_string(dir.join("results.json")).unwrap();
        let md = std::fs::read_to_string(dir.join("results.md")).unwrap();
        assert!(json.contains("\"sim\": \"sim-a\""));
        assert!(json.contains("\"id\": \"xfer\""));
        assert!(md.contains("| sim-a | xfer | p | f | 42 |"));
    }
}
