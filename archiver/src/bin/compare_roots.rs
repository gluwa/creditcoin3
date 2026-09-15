//! Compare Creditcoin V1 merkle roots from a running archiver against an independent
//! Ethereum RPC (e.g. Chainstack). Writes a CSV and a color-coded HTML report.

use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use eth::{simple_merkle_tree, BlockFetchMode, Client};
use serde::Deserialize;
use usc_abi_encoding::common::EncodingVersion;

#[derive(Parser, Debug)]
#[command(
    name = "compare-roots",
    about = "Compare archiver merkle roots against an independent source-chain RPC"
)]
struct Config {
    /// Running archiver HTTP base URL (e.g. http://127.0.0.1:8080).
    #[arg(long, env = "ARCHIVER_URL", default_value = "http://127.0.0.1:8080")]
    archiver_url: String,

    /// Independent HTTP RPC used to recompute roots (Chainstack, etc.).
    #[arg(long, env = "RPC_HTTP", required = true)]
    rpc_http: String,

    /// Fetch mode for the independent RPC. Public providers need `json`.
    #[arg(long, env = "FETCH_MODE", default_value = "json")]
    fetch_mode: BlockFetchMode,

    /// File of block numbers (one per line; `#` comments allowed).
    #[arg(long, default_value = "data/bsc-early-blocks.txt")]
    blocks_file: PathBuf,

    /// Extra comma-separated heights to include.
    #[arg(long, value_delimiter = ',')]
    blocks: Vec<u64>,

    /// Directory for `compare.csv` and `compare.html`.
    #[arg(long, default_value = "./data/bsc-root-compare")]
    out_dir: PathBuf,

    /// Concurrent independent-RPC fetches.
    #[arg(long, default_value = "4")]
    concurrency: NonZeroUsize,
}

#[derive(Debug, Deserialize)]
struct ArchiverRoot {
    block_number: u64,
    merkle_root: String,
}

#[derive(Clone)]
struct Row {
    block: u64,
    tx_count: Option<usize>,
    archiver_root: Option<String>,
    rpc_root: Option<String>,
    note: String,
}

impl Row {
    fn matches(&self) -> Option<bool> {
        match (&self.archiver_root, &self.rpc_root) {
            (Some(a), Some(b)) => Some(normalize_root(a) == normalize_root(b)),
            _ => None,
        }
    }
}

fn normalize_root(s: &str) -> String {
    s.trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_ascii_lowercase()
}

fn parse_blocks_file(path: &Path) -> Result<Vec<u64>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read blocks file {}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let n = line.parse::<u64>().with_context(|| {
            format!(
                "invalid block number on {} line {}: {line}",
                path.display(),
                i + 1
            )
        })?;
        out.push(n);
    }
    Ok(out)
}

fn collect_heights(cfg: &Config) -> Result<Vec<u64>> {
    let mut heights = if cfg.blocks_file.exists() {
        parse_blocks_file(&cfg.blocks_file)?
    } else if cfg.blocks.is_empty() {
        return Err(anyhow!(
            "blocks file {} not found and --blocks was empty",
            cfg.blocks_file.display()
        ));
    } else {
        Vec::new()
    };
    heights.extend(cfg.blocks.iter().copied());
    heights.sort_unstable();
    heights.dedup();
    Ok(heights)
}

async fn archiver_root(
    client: &reqwest::Client,
    base: &str,
    height: u64,
) -> Result<Option<String>> {
    let url = format!(
        "{}/roots?from={height}&to={height}",
        base.trim_end_matches('/')
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|_| anyhow!("archiver request failed"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|_| anyhow!("failed to read archiver response"))?;
    if !status.is_success() {
        if status.as_u16() == 404 {
            return Ok(None);
        }
        return Err(anyhow!("archiver returned HTTP {status}"));
    }
    parse_archiver_root(&body, height).map(Some)
}

fn parse_archiver_root(body: &str, requested_height: u64) -> Result<String> {
    let mut rows: Vec<ArchiverRoot> =
        serde_json::from_str(body).context("invalid archiver JSON response")?;
    anyhow::ensure!(
        rows.len() == 1,
        "archiver returned {} roots; expected exactly one",
        rows.len()
    );
    let row = rows.pop().expect("length checked above");
    anyhow::ensure!(
        row.block_number == requested_height,
        "archiver returned height {}; expected {requested_height}",
        row.block_number
    );
    Ok(row.merkle_root)
}

async fn rpc_root(client: &Client, height: u64) -> Result<(String, usize)> {
    let block = client
        .get_block(height, EncodingVersion::V1)
        .await
        .map_err(|_| anyhow!("RPC block fetch failed"))?;
    let tx_count = block.items().len();
    let root = simple_merkle_tree(&block).root();
    Ok((format!("{root:?}"), tx_count))
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn write_csv(path: &Path, rows: &[Row]) -> Result<()> {
    let mut out = String::from("block,tx_count,archiver_root,rpc_root,match,note\n");
    for row in rows {
        let matched = match row.matches() {
            Some(true) => "yes",
            Some(false) => "NO",
            None => "",
        };
        let tx = row.tx_count.map(|n| n.to_string()).unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            row.block,
            tx,
            csv_escape(row.archiver_root.as_deref().unwrap_or("")),
            csv_escape(row.rpc_root.as_deref().unwrap_or("")),
            matched,
            csv_escape(&row.note),
        ));
    }
    std::fs::write(path, out).with_context(|| format!("write {}", path.display()))
}

fn write_html(path: &Path, rows: &[Row], rpc_label: &str) -> Result<()> {
    let compared = rows.iter().filter(|r| r.matches().is_some()).count();
    let matched = rows.iter().filter(|r| r.matches() == Some(true)).count();
    let mismatched = rows.iter().filter(|r| r.matches() == Some(false)).count();
    let nonempty = rows.iter().filter(|r| r.tx_count.unwrap_or(0) > 0).count();
    let nonempty_ok = rows
        .iter()
        .filter(|r| r.tx_count.unwrap_or(0) > 0 && r.matches() == Some(true))
        .count();

    let mut body = String::new();
    for row in rows {
        let (label, bg) = match row.matches() {
            Some(true) => ("yes", "#c6efce"),
            Some(false) => ("NO", "#ffc7ce"),
            None => ("", "#eeeeee"),
        };
        body.push_str(&format!(
            "<tr style=\"background:{bg}\"><td>{}</td><td>{}</td><td><code>{}</code></td><td><code>{}</code></td><td><b>{}</b></td><td>{}</td></tr>\n",
            row.block,
            row.tx_count.map(|n| n.to_string()).unwrap_or_default(),
            html_escape(row.archiver_root.as_deref().unwrap_or("")),
            html_escape(row.rpc_root.as_deref().unwrap_or("")),
            label,
            html_escape(&row.note),
        ));
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>BSC root compare</title>
<style>
body {{ font-family: sans-serif; margin: 24px; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border: 1px solid #ccc; padding: 6px 8px; text-align: left; }}
code {{ font-size: 12px; }}
</style></head><body>
<h1>BSC archiver vs independent RPC</h1>
<p>Encoding: <b>V1</b>. Independent RPC: <code>{}</code> (redacted label only — do not paste keys into this file).</p>
<p>Compared {compared} · matched {matched} · mismatched {mismatched} · non-empty blocks {nonempty} · non-empty matched {nonempty_ok}</p>
<table>
<thead><tr><th>block</th><th>tx_count</th><th>archiver_root</th><th>rpc_root</th><th>match</th><th>note</th></tr></thead>
<tbody>
{body}
</tbody></table>
</body></html>
"#,
        html_escape(rpc_label),
    );
    std::fs::write(path, html).with_context(|| format!("write {}", path.display()))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn redact_url(url: &str) -> String {
    // Keep only scheme + host (+ port); paths and queries commonly contain API keys.
    match url::Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("unknown");
            match parsed.port() {
                Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
                None => format!("{}://{host}", parsed.scheme()),
            }
        }
        Err(_) => "unparseable-url".to_string(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = Config::parse();
    let heights = collect_heights(&cfg)?;
    anyhow::ensure!(!heights.is_empty(), "no block numbers to compare");

    std::fs::create_dir_all(&cfg.out_dir)?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let eth = Client::new(&cfg.rpc_http, None)
        .await
        .map_err(|_| anyhow!("failed to connect independent RPC"))?
        .with_fetch_mode(cfg.fetch_mode);

    tracing::info!(
        blocks = heights.len(),
        archiver = %redact_url(&cfg.archiver_url),
        rpc = %redact_url(&cfg.rpc_http),
        fetch_mode = %cfg.fetch_mode,
        "comparing roots"
    );

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(cfg.concurrency.get()));

    let mut futs = Vec::new();
    for height in heights {
        let http = http.clone();
        let eth = eth.clone();
        let archiver = cfg.archiver_url.clone();
        let permit = sem.clone();
        futs.push(async move {
            let _permit = permit.acquire().await.expect("semaphore");
            let mut note = String::new();
            let archiver_root = match archiver_root(&http, &archiver, height).await {
                Ok(Some(root)) => Some(root),
                Ok(None) => {
                    note = "archiver missing this height".to_string();
                    None
                }
                Err(err) => {
                    note = format!("archiver error: {err}");
                    None
                }
            };
            let (rpc_root, tx_count) = match rpc_root(&eth, height).await {
                Ok(pair) => (Some(pair.0), Some(pair.1)),
                Err(err) => {
                    if !note.is_empty() {
                        note.push_str("; ");
                    }
                    note.push_str(&format!("rpc error: {err}"));
                    (None, None)
                }
            };
            if let (Some(a), Some(b)) = (&archiver_root, &rpc_root) {
                if normalize_root(a) != normalize_root(b) && note.is_empty() {
                    note = "roots differ".to_string();
                }
            }
            Row {
                block: height,
                tx_count,
                archiver_root,
                rpc_root,
                note,
            }
        });
    }

    let mut rows = futures::future::join_all(futs).await;
    rows.sort_by_key(|r| r.block);

    let csv_path = cfg.out_dir.join("compare.csv");
    let html_path = cfg.out_dir.join("compare.html");
    write_csv(&csv_path, &rows)?;
    write_html(&html_path, &rows, &redact_url(&cfg.rpc_http))?;

    let matched = rows.iter().filter(|r| r.matches() == Some(true)).count();
    let mismatched = rows.iter().filter(|r| r.matches() == Some(false)).count();
    let incomplete = rows
        .iter()
        .filter(|r| r.archiver_root.is_none() || r.rpc_root.is_none())
        .count();
    let nonempty_compared = rows
        .iter()
        .filter(|r| r.tx_count.unwrap_or(0) > 0 && r.matches().is_some())
        .count();
    let nonempty_mismatch = rows
        .iter()
        .filter(|r| r.tx_count.unwrap_or(0) > 0 && r.matches() == Some(false))
        .count();

    tracing::info!(
        csv = %csv_path.display(),
        html = %html_path.display(),
        matched,
        mismatched,
        incomplete,
        nonempty_compared,
        nonempty_mismatch,
        "wrote report"
    );

    let mut failures = Vec::new();
    if mismatched > 0 {
        failures.push(format!(
            "{mismatched} root mismatch(es), {nonempty_mismatch} on non-empty blocks"
        ));
    }
    if incomplete > 0 {
        failures.push(format!("{incomplete} incomplete row(s)"));
    }
    if nonempty_compared == 0 {
        failures.push("no non-empty blocks were successfully compared".to_string());
    }
    if !failures.is_empty() {
        anyhow::bail!("{}", failures.join("; "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archiver_response_requires_one_requested_height() {
        assert_eq!(
            parse_archiver_root(r#"[{"block_number":42,"merkle_root":"0xabc"}]"#, 42).unwrap(),
            "0xabc"
        );
        assert!(parse_archiver_root("[]", 42).is_err());
        assert!(parse_archiver_root(
            r#"[{"block_number":42,"merkle_root":"a"},{"block_number":42,"merkle_root":"b"}]"#,
            42
        )
        .is_err());
        assert!(parse_archiver_root(r#"[{"block_number":41,"merkle_root":"0xabc"}]"#, 42).is_err());
    }

    #[test]
    fn html_escape_covers_root_field_metacharacters() {
        assert_eq!(
            html_escape(r#"<root key="value">'&"#),
            "&lt;root key=&quot;value&quot;&gt;&#39;&amp;"
        );
    }

    #[test]
    fn redacted_urls_drop_paths_queries_and_userinfo() {
        assert_eq!(
            redact_url("https://user:secret@rpc.example:8545/api-key?token=secret"),
            "https://rpc.example:8545"
        );
        assert_eq!(redact_url("not a url"), "unparseable-url");
    }
}
