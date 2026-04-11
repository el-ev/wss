use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STD;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::{ProbeResult, RunError};

const PROBE_TEMPLATE: &str = include_str!("../probe.template.js");

pub(super) fn parse_probe_result(dom_text: &str) -> std::result::Result<ProbeResult, RunError> {
    let re = Regex::new(r#"(?s)<pre id="wss-test-result">(.*?)</pre>"#)
        .expect("probe regex must compile");
    let encoded = re
        .captures(dom_text)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .ok_or_else(|| RunError::error("probe result not found in dumped DOM"))?;
    let encoded = decode_html_entities(&encoded).trim().to_string();
    let bytes = BASE64_STD
        .decode(encoded.as_bytes())
        .map_err(|err| RunError::error(format!("failed to decode probe payload: {}", err)))?;
    serde_json::from_slice::<ProbeResult>(&bytes)
        .map_err(|err| RunError::error(format!("failed to parse probe JSON payload: {}", err)))
}

pub(super) fn infer_materialized_memory_keys(html: &str) -> Vec<String> {
    let re = Regex::new(r"@property\s+(--m[0-9a-f]+)\s*\{").expect("memory key regex must compile");
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    for caps in re.captures_iter(html) {
        if let Some(m) = caps.get(1) {
            let key = m.as_str().to_ascii_lowercase();
            if seen.insert(key.clone()) {
                keys.push(key);
            }
        }
    }
    keys
}

pub(super) fn inject_probe_html(
    html: &str,
    html_path: &Path,
    out_path: &Path,
    memory_keys: &[String],
    max_frames: u64,
    input_bytes: &[u8],
) -> std::result::Result<(), RunError> {
    let marker = "</body>";
    let idx = html.rfind(marker).ok_or_else(|| {
        RunError::error(format!(
            "failed to instrument {}: missing </body>",
            html_path.display()
        ))
    })?;

    let probe_script = make_probe_script(
        memory_keys,
        max_frames,
        input_bytes,
        &infer_getchar_pcs(html),
    );
    let mut instrumented = String::with_capacity(html.len() + probe_script.len() + 8);
    instrumented.push_str(&html[..idx]);
    instrumented.push('\n');
    instrumented.push_str(&probe_script);
    instrumented.push('\n');
    instrumented.push_str(&html[idx..]);

    fs::write(out_path, instrumented).map_err(|err| {
        RunError::error(format!(
            "failed to write instrumented probe HTML '{}': {}",
            out_path.display(),
            err
        ))
    })
}

fn make_probe_script(
    memory_keys: &[String],
    max_frames: u64,
    input_bytes: &[u8],
    getchar_pcs: &[i64],
) -> String {
    PROBE_TEMPLATE
        .replace("__WSS_MAX_FRAMES__", &max_frames.to_string())
        .replace(
            "__WSS_MEMORY_KEYS__",
            &serde_json::to_string(memory_keys).expect("memory keys JSON encoding must work"),
        )
        .replace(
            "__WSS_INPUT_BYTES__",
            &serde_json::to_string(input_bytes).expect("input bytes JSON encoding must work"),
        )
        .replace(
            "__WSS_GETCHAR_PCS__",
            &serde_json::to_string(getchar_pcs).expect("getchar PCs JSON encoding must work"),
        )
}

fn infer_getchar_pcs(html: &str) -> Vec<i64> {
    let re = Regex::new(
        r"style\(--_1pc:\s*(-?\d+)\):\s*--sel\(--ne\(var\(--kb,\s*-1\),\s*-1\),\s*mod\(var\(--kb,\s*-1\),\s*256\),",
    )
    .expect("getchar regex must compile");
    let mut pcs = Vec::new();
    let mut seen = HashSet::new();
    for caps in re.captures_iter(html) {
        if let Some(m) = caps.get(1)
            && let Ok(value) = m.as_str().parse::<i64>()
            && seen.insert(value)
        {
            pcs.push(value);
        }
    }
    pcs
}

fn decode_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}
