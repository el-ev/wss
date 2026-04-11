use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fmt::{self, Display};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::Builder as TempDirBuilder;
use wasmtime::{Caller, Engine, ExternType, Func, Linker, Module, Store, Val};

mod probe;

use probe::{infer_materialized_memory_keys, inject_probe_html, parse_probe_result};

const DEFAULT_CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const TERMINAL_PC_SET_MARKER: &str = "id=\"wss-test-result\"";

const CLANG_ARGS_PREFIX: &[&str] = &[
    "--target=wasm32",
    "-Os",
    "-nostdlib",
    "-mno-implicit-float",
    "-mno-simd128",
    "-fno-exceptions",
    "-mno-bulk-memory",
    "-mno-multivalue",
    "-Wfloat-conversion",
    "-Wl,--gc-sections",
    "-Wl,--no-stack-first",
    "-Wl,--allow-undefined",
    "-Wl,--compress-relocations",
    "-Wl,--strip-all",
    "-Wl,--global-base=4",
];

#[derive(Clone)]
struct EnvConfig {
    root: PathBuf,
    cases_path: PathBuf,
    out_dir: PathBuf,
    chrome_bin: String,
    wss_bin: String,
    normal_timeout_ms: u64,
    max_frames: u64,
    default_wasm_stack_size: u64,
    default_wss_memory_bytes: u64,
    default_wss_stack_slots: u64,
    default_wss_js_clock: bool,
    default_wss_js_coprocessor: bool,
    virtual_time_budget_ms: u64,
    clang_timeout_ms: u64,
    wss_timeout_ms: u64,
    wss_build_timeout_ms: u64,
    dump_timeout_buffer_ms: u64,
    include_lengthy_by_default: bool,
    include_broken_by_default: bool,
    case_retries_by_default: u32,
    case_jobs_by_default: usize,
    case_node_heap_mb_by_default: u64,
}

#[derive(Clone)]
struct CliArgs {
    ids: Vec<String>,
    include_lengthy: bool,
    only_lengthy: bool,
    include_broken: bool,
    only_broken: bool,
    retries: u32,
    jobs: usize,
    case_node_heap_mb: u64,
    json_summary: bool,
    dump_memory_first: bool,
    dump_memory_only: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct TestCase {
    id: String,
    source: String,
    expect: CaseExpect,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    console_input: Option<Value>,
    #[serde(default)]
    js_coprocessor: Option<Value>,
    #[serde(default)]
    js_clock: Option<Value>,
    #[serde(default)]
    reference_types: Option<Value>,
    #[serde(default)]
    wasm_stack_size: Option<Value>,
    #[serde(default)]
    memory_bytes: Option<Value>,
    #[serde(default)]
    stack_slots: Option<Value>,
    #[serde(default)]
    max_frames: Option<Value>,
    #[serde(default)]
    virtual_time_budget_ms: Option<Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct CaseExpect {
    #[serde(default)]
    pc: Option<i64>,
    #[serde(default)]
    ra: Option<String>,
    #[serde(default)]
    rendered_includes: Vec<String>,
    #[serde(default)]
    fb_includes: Vec<String>,
}

#[derive(Clone)]
struct CaseConfig {
    max_frames: u64,
    input_bytes: Vec<u8>,
    reference_types: bool,
    wasm_stack_size: u64,
    memory_bytes: u64,
    stack_slots: u64,
    js_clock: bool,
    js_coprocessor: bool,
    virtual_time_budget_ms: u64,
}

struct CaseArtifacts {
    source_path: PathBuf,
    wasm_path: PathBuf,
    html_path: PathBuf,
    probe_path: PathBuf,
    dom_path: PathBuf,
    memory_dump_path: PathBuf,
    memory_meta_path: PathBuf,
}

#[derive(Clone, Copy)]
struct ProbeBudget {
    budget: u64,
    timeout: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProbeResult {
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    pc: Option<i64>,
    #[serde(default)]
    timeout: Option<bool>,
    #[serde(default)]
    rendered_raw: Option<String>,
    #[serde(default)]
    rendered_normalized: Option<String>,
    #[serde(default)]
    ra: Option<String>,
    #[serde(default)]
    ra_normalized: Option<String>,
    #[serde(default)]
    fb: Option<String>,
    #[serde(default)]
    fb_normalized: Option<String>,
    #[serde(default)]
    memory: HashMap<String, Value>,
}

struct CaseRunOutcome {
    probe_result: ProbeResult,
    failures: Vec<String>,
    dom_path: PathBuf,
    kind: OutcomeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum OutcomeKind {
    Pass,
    Fail,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseOutcome {
    id: String,
    kind: OutcomeKind,
    elapsed_ms: u128,
    attempts_used: u32,
    retries: u32,
    failures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<ProbeResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dom_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunErrorKind {
    Error,
    Timeout,
}

#[derive(Debug, Clone)]
struct RunError {
    kind: RunErrorKind,
    message: String,
    stdout: String,
}

impl RunError {
    fn error(message: impl Into<String>) -> Self {
        Self {
            kind: RunErrorKind::Error,
            message: message.into(),
            stdout: String::new(),
        }
    }

    fn command_failure(
        kind: RunErrorKind,
        context: &str,
        cmd: &str,
        args: &[String],
        stdout: String,
        stderr: String,
        detail: String,
    ) -> Self {
        let message = format_command_failure(context, cmd, args, &stdout, &stderr, &detail);
        Self {
            kind,
            message,
            stdout,
        }
    }
}

impl Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RunError {}

struct CommandOutput {
    stdout: String,
}

#[derive(Default)]
struct HostState {
    input_queue: VecDeque<i32>,
    output_bytes: Vec<u8>,
}

#[derive(Serialize)]
struct ImportMeta {
    module: String,
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct ExportMeta {
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct MemoryMeta {
    id: String,
    source: String,
    memory_dump_path: String,
    memory_meta_path: String,
    byte_length: usize,
    pages: usize,
    ra: Option<String>,
    trap: Option<String>,
    console_input_bytes: Vec<u8>,
    console_output_bytes: Vec<u8>,
    imports: Vec<ImportMeta>,
    exports: Vec<ExportMeta>,
}

pub(crate) fn run() -> Result<i32> {
    let cwd = env::current_dir().context("failed to read current working directory")?;
    let root = find_repo_root(&cwd).context("failed to locate repository root")?;
    let env_cfg = EnvConfig::from_env(root);
    let cli = parse_cli_args(env::args().skip(1).collect(), &env_cfg)?;

    ensure_chrome_path(&env_cfg.chrome_bin)?;
    ensure_wss_binary(&env_cfg)?;
    fs::create_dir_all(&env_cfg.out_dir).with_context(|| {
        format!(
            "failed to create output directory '{}'",
            env_cfg.out_dir.display()
        )
    })?;

    let raw_cases = load_cases(&env_cfg.cases_path)?;
    let resolved = resolve_cases(&raw_cases, &cli.ids)?;

    let mut skipped_lengthy = 0usize;
    let mut skipped_broken = 0usize;
    let mut skipped_unimplemented = 0usize;

    let mut cases = Vec::new();
    for test_case in resolved {
        let lengthy = is_lengthy_case(&test_case);
        let broken = is_broken_case(&test_case);
        let unimplemented = is_unimplemented_case(&test_case);

        if cli.only_lengthy && !lengthy {
            continue;
        }
        if cli.only_broken && !broken {
            continue;
        }

        if cli.ids.is_empty() {
            if !cli.include_lengthy && lengthy {
                skipped_lengthy += 1;
                continue;
            }
            if !cli.include_broken && broken {
                skipped_broken += 1;
                continue;
            }
            if unimplemented {
                skipped_unimplemented += 1;
                continue;
            }
        }

        cases.push(test_case);
    }

    if cases.is_empty() {
        bail!("no cases selected");
    }

    if cli.dump_memory_only {
        dump_memory_for_cases(&cases, &env_cfg)?;
        return Ok(0);
    }

    if cli.json_summary {
        if cases.len() != 1 {
            bail!("--json-summary requires exactly one selected case");
        }
        let outcome =
            evaluate_case_with_retries(&cases[0], cli.retries, cli.dump_memory_first, &env_cfg);
        println!(
            "{}",
            serde_json::to_string(&outcome).context("failed to encode JSON summary")?
        );
        return Ok(if outcome.kind == OutcomeKind::Pass {
            0
        } else {
            1
        });
    }

    println!("Running {} blackbox case(s)...", cases.len());
    if cli.retries > 1 {
        println!("Retries per case: {}", cli.retries);
    }
    if cli.jobs > 1 && cases.len() > 1 {
        println!(
            "Parallel jobs: {} (compat flag case-node-heap-mb={})",
            usize::min(cli.jobs, cases.len()),
            cli.case_node_heap_mb
        );
    }
    if skipped_lengthy > 0 {
        println!(
            "Skipping {} lengthy case(s). Use --include-lengthy to run them.",
            skipped_lengthy
        );
    }
    if skipped_broken > 0 {
        println!(
            "Skipping {} broken case(s). Use --include-broken to run them.",
            skipped_broken
        );
    }
    if skipped_unimplemented > 0 {
        println!(
            "Skipping {} unimplemented case(s). Select by case id to run them.",
            skipped_unimplemented
        );
    }

    let outcomes = if cli.jobs > 1 && cases.len() > 1 {
        run_cases_in_parallel(
            &cases,
            cli.retries,
            cli.jobs,
            cli.dump_memory_first,
            &env_cfg,
        )
    } else {
        run_cases_sequential(&cases, cli.retries, cli.dump_memory_first, &env_cfg)
    };

    let failed = report_summary(cases.len(), &outcomes);
    Ok(if failed { 1 } else { 0 })
}

impl EnvConfig {
    fn from_env(root: PathBuf) -> Self {
        let test_dir = root.join("tests");
        let cases_path = test_dir.join("cases.json");
        let out_dir = test_dir.join("out");

        let default_wss_bin = root.join("target").join("release").join("wss");

        Self {
            root,
            cases_path,
            out_dir,
            chrome_bin: env::var("WSS_CHROME").unwrap_or_else(|_| DEFAULT_CHROME.to_string()),
            wss_bin: env::var("WSS_BIN")
                .unwrap_or_else(|_| default_wss_bin.to_string_lossy().into_owned()),
            normal_timeout_ms: parse_positive_int_env("WSS_NORMAL_TIMEOUT_MS", 15_000),
            max_frames: parse_positive_int_env("WSS_MAX_FRAMES", 15_000),
            default_wasm_stack_size: parse_positive_int_env("WSS_WASM_STACK_SIZE", 256),
            default_wss_memory_bytes: parse_positive_int_env("WSS_MEMORY_BYTES", 1024),
            default_wss_stack_slots: parse_positive_int_env("WSS_STACK_SLOTS", 128),
            default_wss_js_clock: parse_bool_env("WSS_JS_CLOCK", false),
            default_wss_js_coprocessor: false,
            virtual_time_budget_ms: parse_positive_int_env("WSS_VIRTUAL_TIME_BUDGET_MS", 15_000),
            clang_timeout_ms: parse_positive_int_env("WSS_CLANG_TIMEOUT_MS", 3_000),
            wss_timeout_ms: parse_positive_int_env("WSS_TIMEOUT_MS", 3_000),
            wss_build_timeout_ms: parse_positive_int_env("WSS_BUILD_TIMEOUT_MS", 120_000),
            dump_timeout_buffer_ms: parse_positive_int_env("WSS_DUMP_TIMEOUT_BUFFER_MS", 10_000),
            include_lengthy_by_default: parse_bool_env("WSS_INCLUDE_LENGTHY", false),
            include_broken_by_default: parse_bool_env("WSS_INCLUDE_BROKEN", false),
            case_retries_by_default: parse_positive_int_env("WSS_CASE_RETRIES", 1) as u32,
            case_jobs_by_default: parse_positive_int_env("WSS_CASE_JOBS", 1) as usize,
            case_node_heap_mb_by_default: parse_positive_int_env("WSS_CASE_NODE_HEAP_MB", 3072),
        }
    }
}

fn parse_cli_args(argv: Vec<String>, env_cfg: &EnvConfig) -> Result<CliArgs> {
    let mut ids = Vec::new();
    let mut id_set = HashSet::new();
    let mut include_lengthy = env_cfg.include_lengthy_by_default;
    let mut only_lengthy = false;
    let mut include_broken = env_cfg.include_broken_by_default;
    let mut only_broken = false;
    let mut retries = env_cfg.case_retries_by_default;
    let mut jobs = env_cfg.case_jobs_by_default;
    let mut case_node_heap_mb = env_cfg.case_node_heap_mb_by_default;
    let mut json_summary = false;
    let mut dump_memory_first = true;
    let mut dump_memory_only = false;

    let mut i = 0usize;
    while i < argv.len() {
        let arg = &argv[i];
        if arg == "--include-lengthy" {
            include_lengthy = true;
            i += 1;
            continue;
        }
        if arg == "--only-lengthy" {
            include_lengthy = true;
            only_lengthy = true;
            i += 1;
            continue;
        }
        if arg == "--include-broken" {
            include_broken = true;
            i += 1;
            continue;
        }
        if arg == "--only-broken" {
            include_broken = true;
            only_broken = true;
            i += 1;
            continue;
        }
        if let Some((value, consumed)) = try_parse_int_flag("--retries", arg, argv.get(i + 1))? {
            retries = value as u32;
            i += consumed;
            continue;
        }
        if let Some((value, consumed)) = try_parse_int_flag("--jobs", arg, argv.get(i + 1))? {
            jobs = value as usize;
            i += consumed;
            continue;
        }
        if let Some((value, consumed)) =
            try_parse_int_flag("--case-node-heap-mb", arg, argv.get(i + 1))?
        {
            case_node_heap_mb = value;
            i += consumed;
            continue;
        }
        if arg == "--json-summary" {
            json_summary = true;
            i += 1;
            continue;
        }
        if arg == "--dump-memory-first" {
            dump_memory_first = true;
            i += 1;
            continue;
        }
        if arg == "--no-dump-memory" {
            dump_memory_first = false;
            dump_memory_only = false;
            i += 1;
            continue;
        }
        if arg == "--dump-memory-only" {
            dump_memory_first = true;
            dump_memory_only = true;
            i += 1;
            continue;
        }
        if arg.starts_with("--") {
            bail!("unknown flag: {}", arg);
        }
        if id_set.insert(arg.clone()) {
            ids.push(arg.clone());
        }
        i += 1;
    }

    Ok(CliArgs {
        ids,
        include_lengthy,
        only_lengthy,
        include_broken,
        only_broken,
        retries,
        jobs,
        case_node_heap_mb,
        json_summary,
        dump_memory_first,
        dump_memory_only,
    })
}

fn try_parse_int_flag(
    flag: &str,
    arg: &str,
    next_arg: Option<&String>,
) -> Result<Option<(u64, usize)>> {
    if arg == flag {
        let value = next_arg
            .ok_or_else(|| anyhow::anyhow!("{} requires a value", flag))
            .and_then(|v| parse_required_positive_int(v, flag))?;
        return Ok(Some((value, 2)));
    }
    if let Some(value) = arg.strip_prefix(&format!("{}=", flag)) {
        return Ok(Some((parse_required_positive_int(value, flag)?, 1)));
    }
    Ok(None)
}

fn load_cases(cases_path: &Path) -> Result<Vec<TestCase>> {
    let text = fs::read_to_string(cases_path)
        .with_context(|| format!("failed to read cases file '{}'", cases_path.display()))?;
    let raw: Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse JSON '{}'", cases_path.display()))?;
    let arr = raw
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("cases.json must contain an array"))?;

    let mut seen = HashSet::new();
    let mut cases = Vec::with_capacity(arr.len());
    for (index, case_value) in arr.iter().enumerate() {
        let at = format!("cases[{}]", index);
        let obj = case_value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("{} must be an object", at))?;

        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("{}.id must be a non-empty string", at))?;
        if id.trim().is_empty() {
            bail!("{}.id must be a non-empty string", at);
        }
        if !seen.insert(id.to_string()) {
            bail!("duplicate case id \"{}\"", id);
        }

        let source = obj
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("{}.source must be a non-empty string", at))?;
        if source.trim().is_empty() {
            bail!("{}.source must be a non-empty string", at);
        }

        if !obj.get("expect").map(|v| v.is_object()).unwrap_or(false) {
            bail!("{}.expect must be an object", at);
        }

        parse_case_input_bytes(obj.get("console_input"), &format!("{}.console_input", at))?;

        let test_case: TestCase = serde_json::from_value(case_value.clone())
            .with_context(|| format!("{} has invalid structure", at))?;
        cases.push(test_case);
    }

    Ok(cases)
}

fn resolve_cases(raw_cases: &[TestCase], selected_ids: &[String]) -> Result<Vec<TestCase>> {
    if selected_ids.is_empty() {
        return Ok(raw_cases.to_vec());
    }

    let selected: HashSet<&str> = selected_ids.iter().map(String::as_str).collect();
    let cases: Vec<TestCase> = raw_cases
        .iter()
        .filter(|case| selected.contains(case.id.as_str()))
        .cloned()
        .collect();

    let existing: HashSet<&str> = raw_cases.iter().map(|case| case.id.as_str()).collect();
    let missing: Vec<&str> = selected
        .iter()
        .copied()
        .filter(|id| !existing.contains(*id))
        .collect();
    if !missing.is_empty() {
        bail!("unknown case id(s): {}", missing.join(", "));
    }

    Ok(cases)
}

fn has_case_tag(test_case: &TestCase, tag: &str) -> bool {
    test_case.tags.iter().any(|entry| entry == tag)
}

fn is_lengthy_case(test_case: &TestCase) -> bool {
    has_case_tag(test_case, "lengthy")
}

fn is_broken_case(test_case: &TestCase) -> bool {
    has_case_tag(test_case, "broken")
}

fn is_unimplemented_case(test_case: &TestCase) -> bool {
    has_case_tag(test_case, "unimplemented")
}

fn ensure_chrome_path(chrome_bin: &str) -> Result<()> {
    if !chrome_bin.contains(std::path::MAIN_SEPARATOR) {
        return Ok(());
    }
    if Path::new(chrome_bin).exists() {
        return Ok(());
    }
    bail!(
        "Chrome binary not found at {}. Set WSS_CHROME to your Chrome/Chromium path.",
        chrome_bin
    )
}

fn ensure_wss_binary(env_cfg: &EnvConfig) -> Result<()> {
    if Path::new(&env_cfg.wss_bin).exists() {
        return Ok(());
    }
    run_checked(
        "cargo",
        &["build", "--release", "--quiet", "--bin", "wss"],
        &env_cfg.root,
        "[setup] build wss",
        env_cfg.wss_build_timeout_ms,
    )
    .map(|_| ())
    .map_err(|err| anyhow::anyhow!(err.message))
}

fn run_cases_sequential(
    cases: &[TestCase],
    retries: u32,
    use_dump: bool,
    env_cfg: &EnvConfig,
) -> Vec<CaseOutcome> {
    let mut outcomes = Vec::with_capacity(cases.len());
    for (index, test_case) in cases.iter().enumerate() {
        print!(
            "- {} {} ... ",
            format_progress_counter(index + 1, cases.len()),
            test_case.id
        );
        let _ = std::io::stdout().flush();
        let outcome = evaluate_case_with_retries(test_case, retries, use_dump, env_cfg);
        println!("{}", render_outcome_label(&outcome));
        outcomes.push(outcome);
    }
    outcomes
}

fn run_cases_in_parallel(
    cases: &[TestCase],
    retries: u32,
    jobs: usize,
    use_dump: bool,
    env_cfg: &EnvConfig,
) -> Vec<CaseOutcome> {
    let max_jobs = usize::max(1, usize::min(jobs, cases.len()));
    let total_cases = cases.len();
    let cases = Arc::new(cases.to_vec());
    let next_index = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let outcomes: Arc<Mutex<Vec<(usize, CaseOutcome)>>> = Arc::new(Mutex::new(Vec::new()));
    let env_cfg = Arc::new(env_cfg.clone());

    thread::scope(|scope| {
        for _ in 0..max_jobs {
            let cases = Arc::clone(&cases);
            let next_index = Arc::clone(&next_index);
            let completed = Arc::clone(&completed);
            let outcomes = Arc::clone(&outcomes);
            let env_cfg = Arc::clone(&env_cfg);
            scope.spawn(move || {
                loop {
                    let idx = next_index.fetch_add(1, Ordering::SeqCst);
                    if idx >= cases.len() {
                        break;
                    }

                    let test_case = cases[idx].clone();
                    let outcome =
                        evaluate_case_with_retries(&test_case, retries, use_dump, &env_cfg);
                    let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                    println!(
                        "- {} {} ... {}",
                        format_progress_counter(done, total_cases),
                        test_case.id,
                        render_outcome_label(&outcome)
                    );
                    let mut guard = outcomes.lock().expect("outcomes mutex poisoned");
                    guard.push((idx, outcome));
                }
            });
        }
    });

    let mut data = Arc::try_unwrap(outcomes)
        .expect("parallel outcomes still shared")
        .into_inner()
        .expect("outcomes mutex poisoned");
    data.sort_by_key(|(idx, _)| *idx);
    data.into_iter().map(|(_, outcome)| outcome).collect()
}

fn evaluate_case_with_retries(
    test_case: &TestCase,
    retries: u32,
    use_dump: bool,
    env_cfg: &EnvConfig,
) -> CaseOutcome {
    let started_at = Instant::now();
    if use_dump {
        if let Err(err) = prepare_dump_for_case(test_case, env_cfg) {
            return CaseOutcome {
                id: test_case.id.clone(),
                kind: classify_thrown_failure(&err),
                elapsed_ms: started_at.elapsed().as_millis(),
                attempts_used: retries,
                retries,
                failures: vec![err.message],
                result: None,
                dom_path: None,
            };
        }
    }

    let mut attempts_used = 0u32;
    let mut final_failure: Option<CaseOutcome> = None;

    for attempt in 1..=retries {
        attempts_used = attempt;
        match run_case(test_case, use_dump, env_cfg) {
            Ok(outcome) => {
                if outcome.kind == OutcomeKind::Pass {
                    return CaseOutcome {
                        id: test_case.id.clone(),
                        kind: OutcomeKind::Pass,
                        elapsed_ms: started_at.elapsed().as_millis(),
                        attempts_used,
                        retries,
                        failures: Vec::new(),
                        result: None,
                        dom_path: None,
                    };
                }
                final_failure = Some(CaseOutcome {
                    id: test_case.id.clone(),
                    kind: outcome.kind,
                    elapsed_ms: 0,
                    attempts_used,
                    retries,
                    failures: outcome.failures,
                    result: Some(outcome.probe_result),
                    dom_path: Some(outcome.dom_path.to_string_lossy().into_owned()),
                });
            }
            Err(err) => {
                final_failure = Some(CaseOutcome {
                    id: test_case.id.clone(),
                    kind: classify_thrown_failure(&err),
                    elapsed_ms: 0,
                    attempts_used,
                    retries,
                    failures: vec![err.message],
                    result: None,
                    dom_path: None,
                });
            }
        }
    }

    let mut outcome = final_failure.unwrap_or(CaseOutcome {
        id: test_case.id.clone(),
        kind: OutcomeKind::Error,
        elapsed_ms: 0,
        attempts_used,
        retries,
        failures: vec!["internal error: missing failure details after retries".to_string()],
        result: None,
        dom_path: None,
    });
    outcome.elapsed_ms = started_at.elapsed().as_millis();
    outcome.attempts_used = attempts_used;
    outcome
}

fn report_summary(total_cases: usize, outcomes: &[CaseOutcome]) -> bool {
    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut timeout = 0usize;
    let mut error = 0usize;
    let mut failures = Vec::new();

    for outcome in outcomes {
        match outcome.kind {
            OutcomeKind::Pass => pass += 1,
            OutcomeKind::Fail => {
                fail += 1;
                failures.push(outcome);
            }
            OutcomeKind::Timeout => {
                timeout += 1;
                failures.push(outcome);
            }
            OutcomeKind::Error => {
                error += 1;
                failures.push(outcome);
            }
        }
    }

    println!(
        "\nSummary: {}/{} passed, {} failed, {} timed out, {} errors",
        pass, total_cases, fail, timeout, error
    );

    if failures.is_empty() {
        return false;
    }

    for failure in &failures {
        let kind_label = match failure.kind {
            OutcomeKind::Pass => "PASS",
            OutcomeKind::Fail => "FAIL",
            OutcomeKind::Timeout => "TIMEOUT",
            OutcomeKind::Error => "ERROR",
        };
        println!("\n[{}] {}", failure.id, kind_label);
        for item in &failure.failures {
            println!("  - {}", item);
        }
        if let Some(result) = &failure.result {
            println!(
                "  - probe: pc={}, timeout={}, rendered={}",
                result
                    .pc
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "null".to_string()),
                result.timeout.unwrap_or(false),
                serde_json::to_string(&result.rendered_normalized)
                    .unwrap_or_else(|_| "null".to_string())
            );
        }
        if let Some(dom_path) = &failure.dom_path {
            println!("  - dom dump: {}", dom_path);
        }
    }

    println!("\nUnsuccessful tests:");
    let mut failed_ids = Vec::new();
    for failure in &failures {
        let kind_label = match failure.kind {
            OutcomeKind::Pass => "PASS",
            OutcomeKind::Fail => "FAIL",
            OutcomeKind::Timeout => "TIMEOUT",
            OutcomeKind::Error => "ERROR",
        };
        println!("  - {} ({})", failure.id, kind_label);
        failed_ids.push(failure.id.clone());
    }
    println!(
        "\nRerun unsuccessful only: cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml -- {}",
        failed_ids.join(" ")
    );

    true
}

fn render_outcome_label(outcome: &CaseOutcome) -> String {
    let retry_info = if outcome.attempts_used > 1 {
        format!(", attempt {}/{}", outcome.attempts_used, outcome.retries)
    } else {
        String::new()
    };

    match outcome.kind {
        OutcomeKind::Pass => format!("PASS ({}ms{})", outcome.elapsed_ms, retry_info),
        OutcomeKind::Timeout => {
            if outcome.retries > 1 {
                format!(
                    "TIMEOUT after {} attempt(s) ({}ms)",
                    outcome.retries, outcome.elapsed_ms
                )
            } else {
                format!("TIMEOUT ({}ms)", outcome.elapsed_ms)
            }
        }
        OutcomeKind::Error => {
            if outcome.retries > 1 {
                format!(
                    "ERROR after {} attempt(s) ({}ms)",
                    outcome.retries, outcome.elapsed_ms
                )
            } else {
                format!("ERROR ({}ms)", outcome.elapsed_ms)
            }
        }
        OutcomeKind::Fail => {
            if outcome.retries > 1 {
                format!(
                    "FAIL after {} attempt(s) ({}ms)",
                    outcome.retries, outcome.elapsed_ms
                )
            } else {
                format!("FAIL ({}ms)", outcome.elapsed_ms)
            }
        }
    }
}

fn format_progress_counter(current: usize, total: usize) -> String {
    let width = usize::max(2, total.to_string().len());
    format!(
        "[{}/{}]",
        format!("{:0width$}", current, width = width),
        format!("{:0width$}", total, width = width)
    )
}

fn classify_thrown_failure(err: &RunError) -> OutcomeKind {
    match err.kind {
        RunErrorKind::Timeout => OutcomeKind::Timeout,
        RunErrorKind::Error => {
            if err.message.contains("timed out") || err.message.contains("ETIMEDOUT") {
                OutcomeKind::Timeout
            } else {
                OutcomeKind::Error
            }
        }
    }
}

fn run_case(
    test_case: &TestCase,
    use_dump: bool,
    env_cfg: &EnvConfig,
) -> std::result::Result<CaseRunOutcome, RunError> {
    let artifacts = get_case_artifacts(test_case, env_cfg);
    ensure_case_source_exists(&artifacts)?;
    let case_config = build_case_config(test_case, env_cfg)?;

    if !use_dump {
        compile_case_to_wasm(test_case, &artifacts, &case_config, env_cfg)?;
    }

    transpile_case_to_html(test_case, &artifacts, &case_config, env_cfg)?;
    let html = fs::read_to_string(&artifacts.html_path).map_err(|err| {
        RunError::error(format!(
            "[{}] failed to read generated HTML '{}': {}",
            test_case.id,
            artifacts.html_path.display(),
            err
        ))
    })?;

    let materialized_memory_keys = infer_materialized_memory_keys(&html);
    inject_probe_html(
        &html,
        &artifacts.html_path,
        &artifacts.probe_path,
        &materialized_memory_keys,
        case_config.max_frames,
        &case_config.input_bytes,
    )?;

    let probe_budget = build_probe_budget(test_case, case_config.virtual_time_budget_ms, env_cfg);
    let probe_result = run_chrome_probe(test_case, &artifacts, probe_budget, env_cfg)?;

    let mut failures = validate_case(test_case, &probe_result);
    if use_dump {
        failures.extend(compare_dump_with_materialized_runtime(
            test_case,
            &artifacts,
            &materialized_memory_keys,
            &probe_result,
        ));
    }

    let kind = classify_case_failure(&probe_result, &failures);
    Ok(CaseRunOutcome {
        probe_result,
        failures,
        dom_path: artifacts.dom_path,
        kind,
    })
}

fn prepare_dump_for_case(
    test_case: &TestCase,
    env_cfg: &EnvConfig,
) -> std::result::Result<(), RunError> {
    let artifacts = get_case_artifacts(test_case, env_cfg);
    ensure_case_source_exists(&artifacts)?;
    let case_config = build_case_config(test_case, env_cfg)?;
    compile_case_to_wasm(test_case, &artifacts, &case_config, env_cfg)?;
    dump_case_memory(test_case, &artifacts, &case_config)
}

fn dump_memory_for_cases(cases: &[TestCase], env_cfg: &EnvConfig) -> Result<()> {
    for test_case in cases {
        prepare_dump_for_case(test_case, env_cfg).map_err(|err| anyhow::anyhow!(err.message))?;
    }
    Ok(())
}

fn build_probe_budget(
    test_case: &TestCase,
    virtual_time_budget_ms: u64,
    env_cfg: &EnvConfig,
) -> ProbeBudget {
    let lengthy = is_lengthy_case(test_case);
    let normal_budget = u64::min(virtual_time_budget_ms, env_cfg.normal_timeout_ms);
    if lengthy {
        return ProbeBudget {
            budget: virtual_time_budget_ms,
            timeout: virtual_time_budget_ms + env_cfg.dump_timeout_buffer_ms,
        };
    }
    let first_buffer_ms = u64::min(2000, env_cfg.dump_timeout_buffer_ms);
    ProbeBudget {
        budget: normal_budget,
        timeout: normal_budget + first_buffer_ms,
    }
}

fn run_chrome_probe(
    test_case: &TestCase,
    artifacts: &CaseArtifacts,
    probe_budget: ProbeBudget,
    env_cfg: &EnvConfig,
) -> std::result::Result<ProbeResult, RunError> {
    let profile_dir = TempDirBuilder::new()
        .prefix(&format!("{}.chrome-profile-", test_case.id))
        .tempdir_in(&env_cfg.out_dir)
        .map_err(|err| {
            RunError::error(format!(
                "[{}] failed to create temporary Chrome profile dir: {}",
                test_case.id, err
            ))
        })?;

    let chrome_args = vec![
        "--headless=new".to_string(),
        "--disable-gpu".to_string(),
        "--allow-file-access-from-files".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-dev-shm-usage".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--noerrdialogs".to_string(),
        "--test-type".to_string(),
        "--disable-session-crashed-bubble".to_string(),
        format!("--user-data-dir={}", profile_dir.path().display()),
        format!("--virtual-time-budget={}", probe_budget.budget),
        "--dump-dom".to_string(),
        format!("file://{}", artifacts.probe_path.display()),
    ];

    let result = run_checked(
        &env_cfg.chrome_bin,
        &chrome_args,
        &env_cfg.root,
        &format!("[{}] chrome dump-dom", test_case.id),
        probe_budget.timeout,
    );

    match result {
        Ok(output) => write_dom_and_parse_probe(&artifacts.dom_path, &output.stdout),
        Err(err) => {
            if err.kind == RunErrorKind::Timeout && err.stdout.contains(TERMINAL_PC_SET_MARKER) {
                return write_dom_and_parse_probe(&artifacts.dom_path, &err.stdout);
            }
            Err(err)
        }
    }
}

fn write_dom_and_parse_probe(
    dom_path: &Path,
    dom_text: &str,
) -> std::result::Result<ProbeResult, RunError> {
    fs::write(dom_path, dom_text).map_err(|err| {
        RunError::error(format!(
            "failed to write DOM dump '{}': {}",
            dom_path.display(),
            err
        ))
    })?;
    parse_probe_result(dom_text)
}

fn validate_case(test_case: &TestCase, result: &ProbeResult) -> Vec<String> {
    let mut failures = Vec::new();
    let expect = &test_case.expect;

    let probe_error = result.ok == Some(false)
        || result
            .error
            .as_ref()
            .map(|entry| !entry.is_empty())
            .unwrap_or(false);

    if probe_error {
        failures.push(format!(
            "probe error: {}",
            result.error.as_deref().unwrap_or("unknown probe error")
        ));
    }
    if result.timeout.unwrap_or(false) {
        failures.push("probe timed out before terminal PC".to_string());
    }
    if probe_error || result.timeout.unwrap_or(false) {
        return failures;
    }

    if let Some(expected_pc) = expect.pc {
        if result.pc != Some(expected_pc) {
            failures.push(format!(
                "expected pc {}, got {}",
                expected_pc,
                result
                    .pc
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".to_string())
            ));
        }
    }

    if !expect.rendered_includes.is_empty() {
        let haystack = result
            .rendered_normalized
            .as_deref()
            .or(result.rendered_raw.as_deref())
            .unwrap_or_default();
        for needle in &expect.rendered_includes {
            if !haystack.contains(needle) {
                failures.push(format!("missing rendered text: \"{}\"", needle));
            }
        }
    }

    if let Some(expected_ra) = &expect.ra {
        let actual = normalize_hex_content(
            result
                .ra_normalized
                .as_deref()
                .or(result.ra.as_deref())
                .unwrap_or_default(),
        );
        let expected = normalize_hex_content(expected_ra);
        if actual != expected {
            failures.push(format!("expected ra \"{}\", got \"{}\"", expected, actual));
        }
    }

    if !expect.fb_includes.is_empty() {
        let haystack = result
            .fb_normalized
            .as_deref()
            .or(result.fb.as_deref())
            .unwrap_or_default();
        for needle in &expect.fb_includes {
            if !haystack.contains(needle) {
                failures.push(format!("missing framebuffer text: \"{}\"", needle));
            }
        }
    }

    failures
}

fn compare_dump_with_materialized_runtime(
    _test_case: &TestCase,
    artifacts: &CaseArtifacts,
    materialized_memory_keys: &[String],
    result: &ProbeResult,
) -> Vec<String> {
    if result.timeout.unwrap_or(false) || result.ok == Some(false) {
        return Vec::new();
    }
    if materialized_memory_keys.is_empty() {
        return Vec::new();
    }
    if !artifacts.memory_dump_path.exists() {
        return Vec::new();
    }

    let dump = match fs::read(&artifacts.memory_dump_path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };

    let mut mismatches = Vec::new();
    let mut missing_cells = Vec::new();

    for key in materialized_memory_keys {
        let Some(hex_addr) = key.strip_prefix("--m") else {
            continue;
        };
        let Ok(addr) = usize::from_str_radix(hex_addr, 16) else {
            continue;
        };

        let runtime_cell = result.memory.get(key).and_then(|value| match value {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.parse::<i64>().ok(),
            _ => None,
        });

        let Some(runtime_cell) = runtime_cell else {
            missing_cells.push(key.clone());
            continue;
        };

        let runtime_u32 = runtime_cell as u32;
        let runtime_lo = (runtime_u32 & 0xff) as u8;
        let runtime_hi = ((runtime_u32 >> 8) & 0xff) as u8;
        let expected_lo = dump.get(addr).copied();
        let expected_hi = dump.get(addr + 1).copied();

        let Some(expected_lo) = expected_lo else {
            mismatches.push(format!(
                "materialized cell {} starts beyond dump length ({} bytes)",
                key,
                dump.len()
            ));
            continue;
        };

        if runtime_lo != expected_lo {
            mismatches.push(format!(
                "memory byte mismatch at 0x{:04x} ({} lo): dump=0x{:02x} runtime=0x{:02x}",
                addr, key, expected_lo, runtime_lo
            ));
        }

        if let Some(expected_hi) = expected_hi {
            if runtime_hi != expected_hi {
                mismatches.push(format!(
                    "memory byte mismatch at 0x{:04x} ({} hi): dump=0x{:02x} runtime=0x{:02x}",
                    addr + 1,
                    key,
                    expected_hi,
                    runtime_hi
                ));
            }
        }
    }

    let mut failures = Vec::new();
    if !missing_cells.is_empty() {
        let preview: Vec<String> = missing_cells.iter().take(8).cloned().collect();
        let suffix = if missing_cells.len() > 8 { ", ..." } else { "" };
        failures.push(format!(
            "runtime memory missing {} materialized cell(s): {}{}",
            missing_cells.len(),
            preview.join(", "),
            suffix
        ));
    }
    if !mismatches.is_empty() {
        let total_mismatches = mismatches.len();
        let cap = usize::min(32, total_mismatches);
        failures.extend(mismatches.into_iter().take(cap));
        if cap < total_mismatches {
            failures.push(format!(
                "... {} additional byte mismatch(es) omitted",
                total_mismatches - cap
            ));
        }
    }
    failures
}

fn classify_case_failure(probe_result: &ProbeResult, failures: &[String]) -> OutcomeKind {
    if failures.is_empty() {
        return OutcomeKind::Pass;
    }
    if probe_result.ok == Some(false)
        || probe_result
            .error
            .as_ref()
            .map(|entry| !entry.is_empty())
            .unwrap_or(false)
    {
        return OutcomeKind::Error;
    }
    if probe_result.timeout.unwrap_or(false) {
        return OutcomeKind::Timeout;
    }
    OutcomeKind::Fail
}

fn get_case_artifacts(test_case: &TestCase, env_cfg: &EnvConfig) -> CaseArtifacts {
    CaseArtifacts {
        source_path: env_cfg.root.join(&test_case.source),
        wasm_path: env_cfg.out_dir.join(format!("{}.wasm", test_case.id)),
        html_path: env_cfg.out_dir.join(format!("{}.html", test_case.id)),
        probe_path: env_cfg.out_dir.join(format!("{}.probe.html", test_case.id)),
        dom_path: env_cfg.out_dir.join(format!("{}.dom.html", test_case.id)),
        memory_dump_path: env_cfg.out_dir.join(format!("{}.memory.bin", test_case.id)),
        memory_meta_path: env_cfg
            .out_dir
            .join(format!("{}.memory.json", test_case.id)),
    }
}

fn ensure_case_source_exists(artifacts: &CaseArtifacts) -> std::result::Result<(), RunError> {
    if artifacts.source_path.exists() {
        return Ok(());
    }
    Err(RunError::error(format!(
        "missing case source: {}",
        artifacts.source_path.display()
    )))
}

fn build_case_config(
    test_case: &TestCase,
    env_cfg: &EnvConfig,
) -> std::result::Result<CaseConfig, RunError> {
    let js_coprocessor = parse_bool_value(
        test_case.js_coprocessor.as_ref(),
        env_cfg.default_wss_js_coprocessor,
    );
    let requested_js_clock =
        parse_bool_value(test_case.js_clock.as_ref(), env_cfg.default_wss_js_clock);
    let js_clock = requested_js_clock || js_coprocessor;

    let input_bytes = parse_case_input_bytes(
        test_case.console_input.as_ref(),
        &format!("case \"{}\".console_input", test_case.id),
    )
    .map_err(|err| RunError::error(err.to_string()))?;

    Ok(CaseConfig {
        max_frames: parse_positive_int_value(test_case.max_frames.as_ref(), env_cfg.max_frames),
        input_bytes,
        reference_types: parse_bool_value(test_case.reference_types.as_ref(), false),
        wasm_stack_size: parse_positive_int_value(
            test_case.wasm_stack_size.as_ref(),
            env_cfg.default_wasm_stack_size,
        ),
        memory_bytes: parse_positive_int_value(
            test_case.memory_bytes.as_ref(),
            env_cfg.default_wss_memory_bytes,
        ),
        stack_slots: parse_positive_int_value(
            test_case.stack_slots.as_ref(),
            env_cfg.default_wss_stack_slots,
        ),
        js_clock,
        js_coprocessor,
        virtual_time_budget_ms: parse_positive_int_value(
            test_case.virtual_time_budget_ms.as_ref(),
            env_cfg.virtual_time_budget_ms,
        ),
    })
}

fn compile_case_to_wasm(
    test_case: &TestCase,
    artifacts: &CaseArtifacts,
    case_config: &CaseConfig,
    env_cfg: &EnvConfig,
) -> std::result::Result<(), RunError> {
    let args = build_clang_args(
        &artifacts.source_path,
        &artifacts.wasm_path,
        case_config,
        &env_cfg.root,
    );
    run_checked(
        "clang",
        &args,
        &env_cfg.root,
        &format!("[{}] clang compile", test_case.id),
        env_cfg.clang_timeout_ms,
    )
    .map(|_| ())
}

fn transpile_case_to_html(
    test_case: &TestCase,
    artifacts: &CaseArtifacts,
    case_config: &CaseConfig,
    env_cfg: &EnvConfig,
) -> std::result::Result<(), RunError> {
    let args = build_wss_args(
        &artifacts.wasm_path,
        &artifacts.html_path,
        case_config,
        &env_cfg.root,
    );
    run_checked(
        &env_cfg.wss_bin,
        &args,
        &env_cfg.root,
        &format!("[{}] wss run", test_case.id),
        env_cfg.wss_timeout_ms,
    )
    .map(|_| ())
}

fn dump_case_memory(
    test_case: &TestCase,
    artifacts: &CaseArtifacts,
    case_config: &CaseConfig,
) -> std::result::Result<(), RunError> {
    let wasm_bytes = fs::read(&artifacts.wasm_path).map_err(|err| {
        RunError::error(format!(
            "[{}] failed to read wasm '{}': {}",
            test_case.id,
            artifacts.wasm_path.display(),
            err
        ))
    })?;

    let engine = Engine::default();
    let module = Module::new(&engine, &wasm_bytes).map_err(|err| {
        RunError::error(format!(
            "[{}] failed to load wasm module: {}",
            test_case.id, err
        ))
    })?;

    let imports: Vec<ImportMeta> = module
        .imports()
        .map(|entry| ImportMeta {
            module: entry.module().to_string(),
            name: entry.name().to_string(),
            kind: extern_kind_name(entry.ty()),
        })
        .collect();

    let exports: Vec<ExportMeta> = module
        .exports()
        .map(|entry| ExportMeta {
            name: entry.name().to_string(),
            kind: extern_kind_name(entry.ty()),
        })
        .collect();

    let mut store = Store::new(
        &engine,
        HostState {
            input_queue: case_config
                .input_bytes
                .iter()
                .map(|byte| i32::from(*byte))
                .collect(),
            output_bytes: Vec::new(),
        },
    );

    let mut linker = Linker::new(&engine);
    for entry in module.imports() {
        let ty = match entry.ty() {
            ExternType::Func(func_ty) => func_ty,
            _ => {
                return Err(RunError::error(format!(
                    "[{}] unsupported wasm import kind for {}.{}",
                    test_case.id,
                    entry.module(),
                    entry.name()
                )));
            }
        };

        let import_name = entry.name().to_string();
        let func = Func::new(
            &mut store,
            ty,
            move |mut caller: Caller<'_, HostState>, params: &[Val], results: &mut [Val]| {
                let first_arg = params.first().and_then(|v| match v {
                    Val::I32(value) => Some(*value),
                    _ => None,
                });

                let ret = match import_name.as_str() {
                    "getchar" => caller.data_mut().input_queue.pop_front().unwrap_or(-1),
                    "putchar" => {
                        let value = first_arg.unwrap_or(0);
                        caller
                            .data_mut()
                            .output_bytes
                            .push((value as u32 & 0xff) as u8);
                        value
                    }
                    "clock_ms" => 0,
                    _ => first_arg.unwrap_or(0),
                };

                if let Some(slot) = results.first_mut() {
                    *slot = Val::I32(ret);
                }
                for slot in results.iter_mut().skip(1) {
                    *slot = Val::I32(0);
                }

                Ok(())
            },
        );

        linker
            .define(&mut store, entry.module(), entry.name(), func)
            .map_err(|err| {
                RunError::error(format!(
                    "[{}] failed to wire import {}.{}: {}",
                    test_case.id,
                    entry.module(),
                    entry.name(),
                    err
                ))
            })?;
    }

    let instance = linker.instantiate(&mut store, &module).map_err(|err| {
        RunError::error(format!(
            "[{}] failed to instantiate wasm: {}",
            test_case.id, err
        ))
    })?;

    let exported_memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| RunError::error(format!("[{}] expected exported memory", test_case.id)))?;

    let mut ra = None;
    let mut trap = None;

    match instance.get_func(&mut store, "_start") {
        Some(func) => match func.typed::<(), i32>(&store) {
            Ok(start_func) => match start_func.call(&mut store, ()) {
                Ok(value) => ra = Some(format_hex32(value as u32)),
                Err(err) => trap = Some(err.to_string()),
            },
            Err(err) => trap = Some(err.to_string()),
        },
        None => {
            trap = Some("missing exported _start".to_string());
        }
    }

    let final_memory = exported_memory.data(&store).to_vec();
    let output_bytes = store.data().output_bytes.clone();

    fs::write(&artifacts.memory_dump_path, &final_memory).map_err(|err| {
        RunError::error(format!(
            "[{}] failed to write memory dump '{}': {}",
            test_case.id,
            artifacts.memory_dump_path.display(),
            err
        ))
    })?;

    let meta = MemoryMeta {
        id: test_case.id.clone(),
        source: test_case.source.clone(),
        memory_dump_path: artifacts
            .memory_dump_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| artifacts.memory_dump_path.to_string_lossy().into_owned()),
        memory_meta_path: artifacts
            .memory_meta_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| artifacts.memory_meta_path.to_string_lossy().into_owned()),
        byte_length: final_memory.len(),
        pages: final_memory.len() / 65536,
        ra,
        trap,
        console_input_bytes: case_config.input_bytes.clone(),
        console_output_bytes: output_bytes,
        imports,
        exports,
    };

    let json = serde_json::to_string_pretty(&meta).map_err(|err| {
        RunError::error(format!(
            "[{}] failed to encode memory meta JSON: {}",
            test_case.id, err
        ))
    })?;
    fs::write(&artifacts.memory_meta_path, format!("{}\n", json)).map_err(|err| {
        RunError::error(format!(
            "[{}] failed to write memory meta '{}': {}",
            test_case.id,
            artifacts.memory_meta_path.display(),
            err
        ))
    })
}

fn build_clang_args(
    source_path: &Path,
    wasm_path: &Path,
    settings: &CaseConfig,
    root: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = CLANG_ARGS_PREFIX
        .iter()
        .map(|entry| (*entry).to_string())
        .collect();
    args.push(if settings.reference_types {
        "-mreference-types".to_string()
    } else {
        "-mno-reference-types".to_string()
    });
    args.push(format!("-Wl,-z,stack-size={}", settings.wasm_stack_size));
    args.push("-o".to_string());
    args.push(path_arg_from_root(wasm_path, root));
    args.push(path_arg_from_root(source_path, root));
    args
}

fn build_wss_args(
    wasm_path: &Path,
    html_path: &Path,
    settings: &CaseConfig,
    root: &Path,
) -> Vec<String> {
    let mut args = vec![
        path_arg_from_root(wasm_path, root),
        "-o".to_string(),
        path_arg_from_root(html_path, root),
        "--memory-bytes".to_string(),
        settings.memory_bytes.to_string(),
        "--stack-slots".to_string(),
        settings.stack_slots.to_string(),
    ];
    args.push(if settings.js_clock {
        "--js-clock".to_string()
    } else {
        "--no-js-clock".to_string()
    });
    if settings.js_coprocessor {
        args.push("--js-coprocessor".to_string());
    }
    args
}

fn run_checked(
    cmd: &str,
    args: &[impl AsRef<str>],
    cwd: &Path,
    context: &str,
    timeout_ms: u64,
) -> std::result::Result<CommandOutput, RunError> {
    let arg_strings: Vec<String> = args
        .iter()
        .map(|entry| entry.as_ref().to_string())
        .collect();

    let mut child = Command::new(cmd)
        .args(&arg_strings)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            RunError::command_failure(
                RunErrorKind::Error,
                context,
                cmd,
                &arg_strings,
                String::new(),
                String::new(),
                err.to_string(),
            )
        })?;

    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| RunError::error("failed to capture stdout"))?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| RunError::error("failed to capture stderr"))?;

    let stdout_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let timeout = if timeout_ms > 0 {
        Some(Duration::from_millis(timeout_ms))
    } else {
        None
    };
    let started_at = Instant::now();
    let mut timed_out = false;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if timeout.is_some_and(|limit| started_at.elapsed() >= limit) {
                    timed_out = true;
                    let _ = child.kill();
                    let status = child.wait().map_err(|err| {
                        RunError::command_failure(
                            RunErrorKind::Timeout,
                            context,
                            cmd,
                            &arg_strings,
                            String::new(),
                            String::new(),
                            format!("command timed out after {}ms ({})", timeout_ms, err),
                        )
                    })?;
                    break status;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(err) => {
                let stdout = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default())
                    .trim()
                    .to_string();
                let stderr = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default())
                    .trim()
                    .to_string();
                return Err(RunError::command_failure(
                    RunErrorKind::Error,
                    context,
                    cmd,
                    &arg_strings,
                    stdout,
                    stderr,
                    err.to_string(),
                ));
            }
        }
    };

    let stdout = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default())
        .trim()
        .to_string();
    let stderr = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default())
        .trim()
        .to_string();

    if timed_out {
        return Err(RunError::command_failure(
            RunErrorKind::Timeout,
            context,
            cmd,
            &arg_strings,
            stdout,
            stderr,
            format!("command timed out after {}ms", timeout_ms),
        ));
    }

    if !status.success() {
        let mut meta = Vec::new();
        if let Some(code) = status.code() {
            meta.push(format!("status={}", code));
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = status.signal() {
                meta.push(format!("signal={}", sig));
            }
        }
        let meta_suffix = if meta.is_empty() {
            String::new()
        } else {
            format!(" [{}]", meta.join(", "))
        };
        return Err(RunError::command_failure(
            RunErrorKind::Error,
            context,
            cmd,
            &arg_strings,
            stdout,
            stderr,
            format!("command failed{}", meta_suffix),
        ));
    }

    Ok(CommandOutput { stdout })
}

fn parse_positive_int_env(name: &str, fallback: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|raw| parse_positive_int_text(&raw))
        .unwrap_or(fallback)
}

fn parse_bool_env(name: &str, fallback: bool) -> bool {
    match env::var(name) {
        Ok(raw) => parse_bool_text(&raw, fallback),
        Err(_) => fallback,
    }
}

fn parse_positive_int_text(raw: &str) -> Option<u64> {
    let parsed = raw.trim().parse::<u64>().ok()?;
    if parsed == 0 { None } else { Some(parsed) }
}

fn parse_required_positive_int(raw: &str, context: &str) -> Result<u64> {
    let text = raw.trim();
    if !text.chars().all(|ch| ch.is_ascii_digit()) || text.is_empty() {
        bail!("{} must be a positive integer", context);
    }
    let parsed = text.parse::<u64>().context("integer parsing failure")?;
    if parsed == 0 {
        bail!("{} must be a positive integer", context);
    }
    Ok(parsed)
}

fn parse_bool_text(raw: &str, fallback: bool) -> bool {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return fallback;
    }
    match normalized.as_str() {
        "1" | "true" | "yes" | "y" | "on" => true,
        "0" | "false" | "no" | "n" | "off" => false,
        _ => fallback,
    }
}

fn parse_bool_value(raw: Option<&Value>, fallback: bool) -> bool {
    match raw {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().map(|n| n != 0).unwrap_or(fallback),
        Some(Value::String(value)) => parse_bool_text(value, fallback),
        Some(Value::Null) | None => fallback,
        _ => fallback,
    }
}

fn parse_positive_int_value(raw: Option<&Value>, fallback: u64) -> u64 {
    match raw {
        Some(Value::Number(value)) => value.as_u64().filter(|n| *n > 0).unwrap_or(fallback),
        Some(Value::String(value)) => parse_positive_int_text(value).unwrap_or(fallback),
        _ => fallback,
    }
}

fn parse_case_input_bytes(raw: Option<&Value>, context: &str) -> Result<Vec<u8>> {
    match raw {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(value)) => Ok(value.as_bytes().to_vec()),
        Some(Value::Array(entries)) => {
            let mut bytes = Vec::with_capacity(entries.len());
            for (index, entry) in entries.iter().enumerate() {
                let Value::Number(number) = entry else {
                    bail!("{}[{}] must be an integer in [0, 255]", context, index);
                };
                let Some(value) = number.as_i64() else {
                    bail!("{}[{}] must be an integer in [0, 255]", context, index);
                };
                if !(0..=255).contains(&value) {
                    bail!("{}[{}] must be an integer in [0, 255]", context, index);
                }
                bytes.push(value as u8);
            }
            Ok(bytes)
        }
        _ => bail!("{} must be a string or an array of byte values", context),
    }
}

fn normalize_hex_content(raw: &str) -> String {
    let cleaned = raw
        .to_ascii_lowercase()
        .replace('"', "")
        .split_whitespace()
        .collect::<String>();

    let re = Regex::new(r"0x[0-9a-f]+$").expect("hex regex must compile");
    if re.is_match(&cleaned) {
        return cleaned;
    }

    let find = Regex::new(r"0x[0-9a-f]+").expect("hex-find regex must compile");
    find.find(&cleaned)
        .map(|m| m.as_str().to_string())
        .unwrap_or(cleaned)
}

fn path_arg_from_root(abs_path: &Path, root: &Path) -> String {
    match abs_path.strip_prefix(root) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().into_owned(),
        _ => abs_path.to_string_lossy().into_owned(),
    }
}

fn format_hex32(value: u32) -> String {
    format!("0x{:08x}", value)
}

fn format_command(cmd: &str, args: &[String]) -> String {
    if args.is_empty() {
        cmd.to_string()
    } else {
        format!("{} {}", cmd, args.join(" "))
    }
}

fn format_command_failure(
    context: &str,
    cmd: &str,
    args: &[String],
    stdout: &str,
    stderr: &str,
    detail: &str,
) -> String {
    let mut parts = vec![format!(
        "{}: {} ({})",
        context,
        detail,
        format_command(cmd, args)
    )];
    if !stdout.is_empty() {
        parts.push(format!("stdout:\n{}", stdout));
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr:\n{}", stderr));
    }
    parts.join("\n\n")
}

fn extern_kind_name(ty: ExternType) -> String {
    match ty {
        ExternType::Func(_) => "function".to_string(),
        ExternType::Global(_) => "global".to_string(),
        ExternType::Table(_) => "table".to_string(),
        ExternType::Memory(_) => "memory".to_string(),
        ExternType::Tag(_) => "tag".to_string(),
    }
}

fn find_repo_root(start: &Path) -> Result<PathBuf> {
    for dir in start.ancestors() {
        let cargo = dir.join("Cargo.toml");
        let cases = dir.join("tests").join("cases.json");
        if cargo.exists() && cases.exists() {
            return Ok(dir.to_path_buf());
        }
    }
    bail!(
        "could not find repository root from '{}': missing Cargo.toml and tests/cases.json",
        start.display()
    )
}
