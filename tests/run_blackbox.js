#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawn, spawnSync } = require("node:child_process");

const ROOT = path.resolve(__dirname, "..");
const TEST_DIR = __dirname;
const CASES_PATH = path.join(TEST_DIR, "cases.json");
const OUT_DIR = path.join(TEST_DIR, "out");
const DEFAULT_WSS_BIN = path.join(ROOT, "target", "release", "wss");

const DEFAULT_CHROME =
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const CHROME_BIN = process.env.WSS_CHROME || DEFAULT_CHROME;
const NORMAL_TIMEOUT_MS = parsePositiveInt(process.env.WSS_NORMAL_TIMEOUT_MS, 15000);
const MAX_FRAMES = parsePositiveInt(process.env.WSS_MAX_FRAMES, 15000);
const DEFAULT_WASM_STACK_SIZE = parsePositiveInt(
  process.env.WSS_WASM_STACK_SIZE,
  256
);
const DEFAULT_WSS_MEMORY_BYTES = parsePositiveInt(
  process.env.WSS_MEMORY_BYTES,
  1024
);
const DEFAULT_WSS_STACK_SLOTS = parsePositiveInt(
  process.env.WSS_STACK_SLOTS,
  128
);
const DEFAULT_WSS_JS_CLOCK = parseBool(process.env.WSS_JS_CLOCK, false);
const DEFAULT_WSS_JS_COPROCESSOR = false;
const VIRTUAL_TIME_BUDGET_MS = parsePositiveInt(
  process.env.WSS_VIRTUAL_TIME_BUDGET_MS,
  NORMAL_TIMEOUT_MS
);
const CLANG_TIMEOUT_MS = parsePositiveInt(process.env.WSS_CLANG_TIMEOUT_MS, 3000);
const WSS_TIMEOUT_MS = parsePositiveInt(process.env.WSS_TIMEOUT_MS, 3000);
const WSS_BUILD_TIMEOUT_MS = parsePositiveInt(process.env.WSS_BUILD_TIMEOUT_MS, 120000);
const DUMP_TIMEOUT_BUFFER_MS = parsePositiveInt(
  process.env.WSS_DUMP_TIMEOUT_BUFFER_MS,
  10000
);
const WSS_BIN = process.env.WSS_BIN || DEFAULT_WSS_BIN;
const INCLUDE_LENGTHY_BY_DEFAULT = parseBool(process.env.WSS_INCLUDE_LENGTHY, false);
const INCLUDE_BROKEN_BY_DEFAULT = parseBool(process.env.WSS_INCLUDE_BROKEN, false);
const CASE_RETRIES_BY_DEFAULT = parsePositiveInt(process.env.WSS_CASE_RETRIES, 1);
const CASE_JOBS_BY_DEFAULT = parsePositiveInt(process.env.WSS_CASE_JOBS, 1);
const CASE_NODE_HEAP_MB_BY_DEFAULT = parsePositiveInt(
  process.env.WSS_CASE_NODE_HEAP_MB,
  3072
);

const CLANG_ARGS_PREFIX = [
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

function parsePositiveInt(raw, fallback) {
  const parsed = Number.parseInt(String(raw ?? ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
}

function parseBool(raw, fallback) {
  if (raw === undefined || raw === null || raw === "") return fallback;
  const normalized = String(raw).trim().toLowerCase();
  if (["1", "true", "yes", "y", "on"].includes(normalized)) return true;
  if (["0", "false", "no", "n", "off"].includes(normalized)) return false;
  return fallback;
}

function parseRequiredPositiveInt(raw, context) {
  const text = String(raw ?? "").trim();
  if (!/^\d+$/.test(text)) {
    throw new Error(`${context} must be a positive integer`);
  }
  const parsed = Number.parseInt(text, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${context} must be a positive integer`);
  }
  return parsed;
}

function parseCaseInputBytes(raw, context) {
  if (raw === undefined || raw === null) return [];
  if (typeof raw === "string") {
    return [...Buffer.from(raw, "utf8")];
  }
  if (!Array.isArray(raw)) {
    throw new Error(`${context} must be a string or an array of byte values`);
  }

  return raw.map((value, index) => {
    if (!Number.isInteger(value) || value < 0 || value > 255) {
      throw new Error(`${context}[${index}] must be an integer in [0, 255]`);
    }
    return value;
  });
}

function normalizeHexContent(raw) {
  const cleaned = String(raw ?? "")
    .toLowerCase()
    .replaceAll("\"", "")
    .replace(/\s+/g, "");
  const m = cleaned.match(/0x[0-9a-f]+/);
  return m ? m[0] : cleaned;
}

function decodeHtmlEntities(text) {
  return text
    .replaceAll("&amp;", "&")
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", "\"")
    .replaceAll("&#39;", "'");
}

function formatCommand(cmd, args) {
  return `${cmd} ${args.join(" ")}`;
}

function formatCommandFailure(context, cmd, args, stdout, stderr, detail) {
  return [
    `${context}: ${detail} (${formatCommand(cmd, args)})`,
    stdout ? `stdout:\n${stdout}` : "",
    stderr ? `stderr:\n${stderr}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
}

function makeTaggedError(kind, message, extra = {}) {
  const err = new Error(message);
  err.kind = kind;
  Object.assign(err, extra);
  return err;
}

function isSpawnTimeoutError(error) {
  if (!error) return false;
  const code = typeof error.code === "string" ? error.code.toUpperCase() : "";
  const message = typeof error.message === "string" ? error.message : "";
  return code === "ETIMEDOUT" || /timed out/i.test(message);
}

function runChecked(cmd, args, cwd, context, timeoutMs = 0) {
  const result = spawnSync(cmd, args, {
    cwd,
    encoding: "utf8",
    maxBuffer: 128 * 1024 * 1024,
    timeout: timeoutMs > 0 ? timeoutMs : undefined,
    killSignal: "SIGKILL",
  });

  if (result.error) {
    const stdout = (result.stdout || "").trim();
    const stderr = (result.stderr || "").trim();
    const timeout = timeoutMs > 0 && isSpawnTimeoutError(result.error);
    const detail = timeout
      ? `command timed out after ${timeoutMs}ms`
      : result.error.message;
    throw makeTaggedError(
      timeout ? "timeout" : "error",
      formatCommandFailure(context, cmd, args, stdout, stderr, detail),
      {
        stdout,
        stderr,
        status: result.status,
        signal: result.signal,
      }
    );
  }

  if (result.status !== 0) {
    const stdout = (result.stdout || "").trim();
    const stderr = (result.stderr || "").trim();
    const meta = [];
    if (typeof result.status === "number") meta.push(`status=${result.status}`);
    if (result.signal) meta.push(`signal=${result.signal}`);
    const metaSuffix = meta.length > 0 ? ` [${meta.join(", ")}]` : "";
    const timeout =
      timeoutMs > 0 && result.status === null && String(result.signal || "") === "SIGKILL";
    const detail = timeout
      ? `command timed out after ${timeoutMs}ms${metaSuffix}`
      : `command failed${metaSuffix}`;
    throw makeTaggedError(
      timeout ? "timeout" : "error",
      formatCommandFailure(
        context,
        cmd,
        args,
        stdout,
        stderr,
        detail
      ),
      {
        stdout,
        stderr,
        status: result.status,
        signal: result.signal,
      }
    );
  }

  return result;
}

function pathArgFromRoot(absPath) {
  const rel = path.relative(ROOT, absPath);
  if (!rel || rel.startsWith("..")) return absPath;
  return rel;
}

function getCaseArtifacts(testCase) {
  const id = testCase.id;
  return {
    sourcePath: path.resolve(ROOT, testCase.source),
    wasmPath: path.join(OUT_DIR, `${id}.wasm`),
    htmlPath: path.join(OUT_DIR, `${id}.html`),
    probePath: path.join(OUT_DIR, `${id}.probe.html`),
    domPath: path.join(OUT_DIR, `${id}.dom.html`),
    memoryDumpPath: path.join(OUT_DIR, `${id}.memory.bin`),
    memoryMetaPath: path.join(OUT_DIR, `${id}.memory.json`),
  };
}

function formatHex32(value) {
  return `0x${(value >>> 0).toString(16).padStart(8, "0")}`;
}

function buildClangArgs(sourcePath, wasmPath, settings) {
  const referenceTypesFlag = settings.referenceTypes
    ? "-mreference-types"
    : "-mno-reference-types";
  return [
    ...CLANG_ARGS_PREFIX,
    referenceTypesFlag,
    `-Wl,-z,stack-size=${settings.wasmStackSize}`,
    "-o",
    pathArgFromRoot(wasmPath),
    pathArgFromRoot(sourcePath),
  ];
}

function buildWssArgs(wasmPath, htmlPath, settings) {
  const args = [
    pathArgFromRoot(wasmPath),
    "-o",
    pathArgFromRoot(htmlPath),
    "--memory-bytes",
    String(settings.memoryBytes),
    "--stack-slots",
    String(settings.stackSlots),
  ];
  args.push(settings.jsClock ? "--js-clock" : "--no-js-clock");
  if (settings.jsCoprocessor) {
    args.push("--js-coprocessor");
  }
  return args;
}

function makeProbeScript(memoryKeys, maxFrames, inputBytes, getcharPcs) {
  return `<script id="wss-test-probe">
(() => {
  const terminalPcs = new Set([-1, -2, -3, -4, -5]);
  const maxFrames = ${maxFrames};
  const memoryKeys = ${JSON.stringify(memoryKeys)};
  const inputQueue = ${JSON.stringify(inputBytes)};
  const getcharPcs = new Set(${JSON.stringify(getcharPcs)});
  const noInputValue = -1;

  function readInt(style, name, fallback) {
    const parsed = Number.parseInt(style.getPropertyValue(name), 10);
    if (Number.isNaN(parsed)) return fallback;
    return parsed;
  }

  function readByte(style, name) {
    return readInt(style, name, 0) & 0xff;
  }

  function readCopWord(style, prefix) {
    return (
      readByte(style, prefix + "0") |
      (readByte(style, prefix + "1") << 8) |
      (readByte(style, prefix + "2") << 16) |
      (readByte(style, prefix + "3") << 24)
    ) >>> 0;
  }

  function writeCopOutputBytes(terminalEl, b0, b1, b2, b3) {
    terminalEl.style.setProperty("--cop_o0", String(b0), "important");
    terminalEl.style.setProperty("--cop_o1", String(b1), "important");
    terminalEl.style.setProperty("--cop_o2", String(b2), "important");
    terminalEl.style.setProperty("--cop_o3", String(b3), "important");
  }

  function writeCopOutputWord(terminalEl, value) {
    const word = value >>> 0;
    writeCopOutputBytes(
      terminalEl,
      word & 0xff,
      (word >>> 8) & 0xff,
      (word >>> 16) & 0xff,
      (word >>> 24) & 0xff
    );
  }

  function popcnt32(value) {
    let v = value >>> 0;
    let count = 0;
    while (v !== 0) {
      v &= v - 1;
      count += 1;
    }
    return count >>> 0;
  }

  function runCoprocessorStep(terminalEl, style) {
    const op = readInt(style, "--cop_op", 0);
    if (op === 0) {
      writeCopOutputWord(terminalEl, 0);
      return;
    }

    const lhsU = readCopWord(style, "--cop_a");
    const rhsU = readCopWord(style, "--cop_b");
    const lhsS = lhsU | 0;
    const rhsS = rhsU | 0;

    switch (op) {
      case 1: // div_u32
        if (rhsU === 0) return writeCopOutputBytes(terminalEl, -1, 0, 0, 0);
        return writeCopOutputWord(terminalEl, Math.trunc(lhsU / rhsU));
      case 2: // rem_u32
        if (rhsU === 0) return writeCopOutputBytes(terminalEl, -1, 0, 0, 0);
        return writeCopOutputWord(terminalEl, lhsU % rhsU);
      case 3: // div_s32
        if (rhsU === 0) return writeCopOutputBytes(terminalEl, -1, 0, 0, 0);
        return writeCopOutputWord(terminalEl, (lhsS / rhsS) | 0);
      case 4: // rem_s32
        if (rhsU === 0) return writeCopOutputBytes(terminalEl, -1, 0, 0, 0);
        return writeCopOutputWord(terminalEl, (lhsS % rhsS) | 0);
      case 5: {
        const shift = rhsU & 31; // shl_32
        return writeCopOutputWord(terminalEl, lhsU << shift);
      }
      case 6: {
        const shift = rhsU & 31; // shr_u32
        return writeCopOutputWord(terminalEl, lhsU >>> shift);
      }
      case 7: {
        const shift = rhsU & 31; // shr_s32
        return writeCopOutputWord(terminalEl, lhsS >> shift);
      }
      case 8: {
        const shift = rhsU & 31; // rotl_32
        return writeCopOutputWord(terminalEl, (lhsU << shift) | (lhsU >>> ((32 - shift) & 31)));
      }
      case 9: {
        const shift = rhsU & 31; // rotr_32
        return writeCopOutputWord(terminalEl, (lhsU >>> shift) | (lhsU << ((32 - shift) & 31)));
      }
      case 10: // clz_32
        return writeCopOutputWord(terminalEl, Math.clz32(lhsU));
      case 11: // ctz_32
        return writeCopOutputWord(
          terminalEl,
          lhsU === 0 ? 32 : 31 - Math.clz32((lhsU & -lhsU) >>> 0)
        );
      case 12: // popcnt_32
        return writeCopOutputWord(terminalEl, popcnt32(lhsU));
      default:
        return writeCopOutputWord(terminalEl, 0);
    }
  }

  function normalizeContent(raw) {
    if (!raw || raw === "none") return "";
    const text = String(raw).trim();
    if (!text) return "";

    const decodeOne = (quoted) =>
      quoted
        .replace(/^"/, "")
        .replace(/"$/, "")
        .replace(/\\\\A\\s?/gi, "\\n")
        .replace(/\\\\a\\s?/g, "\\n")
        .replace(/\\\\n/g, "\\n")
        .replace(/\\\\"/g, '"')
        .replace(/\\\\\\\\/g, "\\\\");

    const parts = text.match(/"(?:\\\\.|[^"\\\\])*"/g);
    if (parts && text.replace(/"(?:\\\\.|[^"\\\\])*"/g, "").trim() === "") {
      return parts.map(decodeOne).join("");
    }

    return decodeOne(text);
  }

  function collectMemory(style) {
    const out = {};
    for (const key of memoryKeys) {
      const raw = style.getPropertyValue(key).trim();
      if (!raw) continue;
      const parsed = Number.parseInt(raw, 10);
      out[key] = Number.isNaN(parsed) ? raw : parsed;
    }
    return out;
  }

  function toBase64Utf8(text) {
    const bytes = new TextEncoder().encode(text);
    let bin = "";
    for (const byte of bytes) bin += String.fromCharCode(byte);
    return btoa(bin);
  }

  function emit(payload) {
    let pre = document.getElementById("wss-test-result");
    if (!pre) {
      pre = document.createElement("pre");
      pre.id = "wss-test-result";
      document.body.appendChild(pre);
    }
    pre.textContent = toBase64Utf8(JSON.stringify(payload));
  }

  function readState() {
    const clk = document.querySelector(".clk");
    const terminal = document.querySelector(".terminal");
    const screen = document.querySelector(".screen");
    if (!clk || !terminal || !screen) {
      return {
        ok: false,
        error: "missing runtime nodes",
        pc: null,
        rendered_raw: "",
        rendered_normalized: "",
        memory: {}
      };
    }

    const termStyle = getComputedStyle(terminal);
    const pcNow = readInt(termStyle, "--_1pc", Number.NaN);
    const pc = readInt(termStyle, "--pc", pcNow);
    const renderedRaw = getComputedStyle(screen, "::after").content || "";
    const raRaw = termStyle.getPropertyValue("--ra").trim();
    const fbRaw = termStyle.getPropertyValue("--fb").trim();
    return {
      ok: true,
      pc,
      pc_now: pcNow,
      pc_raw: termStyle.getPropertyValue("--pc").trim() || termStyle.getPropertyValue("--_1pc").trim(),
      fb: fbRaw,
      fb_normalized: normalizeContent(fbRaw),
      ra: raRaw,
      ra_normalized: normalizeContent(raRaw),
      rendered_raw: renderedRaw,
      rendered_normalized: normalizeContent(renderedRaw),
      memory: collectMemory(termStyle)
    };
  }

  function tickInstruction(clkEl, terminalEl, clockRef) {
    for (let i = 0; i < 4; i++) {
      clkEl.style.setProperty("--clk", String(clockRef.value), "important");
      if (clockRef.value === 2) {
        const style = getComputedStyle(terminalEl);
        if (style.getPropertyValue("--cop_op").trim() !== "") {
          runCoprocessorStep(terminalEl, style);
        }
      }
      clockRef.value = (clockRef.value + 1) & 3;
      getComputedStyle(terminalEl).getPropertyValue("--_1pc");
    }
  }

  function stageInputByte(clkEl, state) {
    const pc = Number.isInteger(state.pc_now) ? state.pc_now : null;
    if (pc !== null && getcharPcs.has(pc) && inputQueue.length > 0) {
      const next = inputQueue.shift();
      clkEl.style.setProperty("--kb", String(next), "important");
      return next;
    }
    clkEl.style.setProperty("--kb", String(noInputValue), "important");
    return null;
  }

  const clkEl = document.querySelector(".clk");
  const terminalEl = document.querySelector(".terminal");
  const clockRef = { value: 0 };
  if (clkEl) {
    // The runtime clock animation keeps virtual time alive even after the probe
    // reaches a terminal PC. The probe drives --clk manually, so disable the
    // ambient animation while testing.
    clkEl.style.setProperty("animation", "none", "important");
    clkEl.style.setProperty("--kb", String(noInputValue), "important");
  }
  let frames = 0;
  let state = readState();
  while (
    clkEl &&
    terminalEl &&
    (!Number.isInteger(state.pc) || !terminalPcs.has(state.pc)) &&
    frames < maxFrames
  ) {
    stageInputByte(clkEl, state);
    tickInstruction(clkEl, terminalEl, clockRef);
    clkEl.style.setProperty("--kb", String(noInputValue), "important");
    frames += 1;
    state = readState();
  }

  // Chrome headless keeps advancing virtual time while CSS animations or
  // transitions remain active anywhere on the page. The probe has already
  // captured the final machine state, so freeze the document before emitting
  // the result marker.
  const freezeStyle = document.createElement("style");
  freezeStyle.id = "wss-test-freeze";
  freezeStyle.textContent = "*, *::before, *::after { animation: none !important; transition: none !important; }";
  document.head.appendChild(freezeStyle);
  if (clkEl) {
    clkEl.style.setProperty("animation", "none", "important");
  }
  if (terminalEl) {
    terminalEl.style.setProperty("animation", "none", "important");
  }
  getComputedStyle(document.body).getPropertyValue("display");

  emit({
    ...state,
    done: Number.isInteger(state.pc) && terminalPcs.has(state.pc),
    timeout: !(Number.isInteger(state.pc) && terminalPcs.has(state.pc)),
    frames,
    remaining_input_bytes: inputQueue.length
  });
})();
</script>`;
}

function injectProbeHtml(htmlPath, outPath, memoryKeys, maxFrames, inputBytes) {
  const html = fs.readFileSync(htmlPath, "utf8");
  const marker = "</body>";
  const idx = html.lastIndexOf(marker);
  if (idx === -1) {
    throw new Error(`failed to instrument ${htmlPath}: missing </body>`);
  }

  const probeScript = makeProbeScript(
    memoryKeys,
    maxFrames,
    inputBytes,
    inferGetcharPcs(html)
  );
  const instrumented = `${html.slice(0, idx)}\n${probeScript}\n${html.slice(idx)}`;
  fs.writeFileSync(outPath, instrumented);
}

function parseProbeResult(domText) {
  const match = domText.match(/<pre id="wss-test-result">([\s\S]*?)<\/pre>/);
  if (!match) {
    throw new Error("probe result not found in dumped DOM");
  }
  const encoded = decodeHtmlEntities(match[1]).trim();
  const json = Buffer.from(encoded, "base64").toString("utf8");
  return JSON.parse(json);
}

function inferGetcharPcs(html) {
  const pcs = new Set();
  const re =
    /style\(--_1pc:\s*(-?\d+)\):\s*--sel\(--ne\(var\(--kb,\s*-1\),\s*-1\),\s*mod\(var\(--kb,\s*-1\),\s*256\),/g;
  let match;
  while ((match = re.exec(html)) !== null) {
    const parsed = Number.parseInt(match[1], 10);
    if (Number.isFinite(parsed)) {
      pcs.add(parsed);
    }
  }
  return [...pcs];
}

function inferMaterializedMemoryKeys(html) {
  const keys = [];
  const seen = new Set();
  const re = /@property\s+(--m[0-9a-f]+)\s*\{/gi;
  let match;
  while ((match = re.exec(html)) !== null) {
    const key = match[1].toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    keys.push(key);
  }
  return keys;
}

function validateCase(testCase, result) {
  const failures = [];
  const expect = testCase.expect || {};
  const probeError =
    result && (result.ok === false || (typeof result.error === "string" && result.error !== ""));

  if (probeError) {
    failures.push(`probe error: ${result.error || "unknown probe error"}`);
  }
  if (result.timeout) {
    failures.push("probe timed out before terminal PC");
  }
  if (probeError || result.timeout) {
    return failures;
  }

  if (expect.pc !== undefined && result.pc !== expect.pc) {
    failures.push(`expected pc ${expect.pc}, got ${result.pc}`);
  }
  if (Array.isArray(expect.rendered_includes)) {
    const haystack = String(result.rendered_normalized || result.rendered_raw || "");
    for (const needle of expect.rendered_includes) {
      if (!haystack.includes(needle)) {
        failures.push(`missing rendered text: "${needle}"`);
      }
    }
  }
  if (expect.ra !== undefined) {
    const actual = normalizeHexContent(result.ra_normalized || result.ra || "");
    const expected = normalizeHexContent(expect.ra);
    if (actual !== expected) {
      failures.push(`expected ra "${expected}", got "${actual}"`);
    }
  }
  if (Array.isArray(expect.fb_includes)) {
    const haystack = String(result.fb_normalized || result.fb || "");
    for (const needle of expect.fb_includes) {
      if (!haystack.includes(needle)) {
        failures.push(`missing framebuffer text: "${needle}"`);
      }
    }
  }

  return failures;
}

function compareDumpWithMaterializedRuntime(testCase, artifacts, materializedMemoryKeys, result) {
  if (!result || result.timeout || result.ok === false) return [];
  if (!Array.isArray(materializedMemoryKeys) || materializedMemoryKeys.length === 0) return [];
  if (!fs.existsSync(artifacts.memoryDumpPath)) return [];

  const dump = fs.readFileSync(artifacts.memoryDumpPath);
  const mismatches = [];
  const missingCells = [];

  for (const key of materializedMemoryKeys) {
    const match = /^--m([0-9a-f]+)$/i.exec(key);
    if (!match) continue;

    const addr = Number.parseInt(match[1], 16);
    const runtimeCell = result.memory ? result.memory[key] : undefined;
    if (!Number.isInteger(runtimeCell)) {
      missingCells.push(key);
      continue;
    }

    const runtimeLo = runtimeCell & 0xff;
    const runtimeHi = (runtimeCell >>> 8) & 0xff;
    const expectedLo = dump[addr];
    const expectedHi = dump[addr + 1];

    if (expectedLo === undefined) {
      mismatches.push(
        `materialized cell ${key} starts beyond dump length (${dump.length} bytes)`
      );
      continue;
    }
    if (runtimeLo !== expectedLo) {
      mismatches.push(
        `memory byte mismatch at 0x${addr.toString(16).padStart(4, "0")} (${key} lo): dump=0x${expectedLo
          .toString(16)
          .padStart(2, "0")} runtime=0x${runtimeLo.toString(16).padStart(2, "0")}`
      );
    }
    if (expectedHi !== undefined && runtimeHi !== expectedHi) {
      mismatches.push(
        `memory byte mismatch at 0x${(addr + 1)
          .toString(16)
          .padStart(4, "0")} (${key} hi): dump=0x${expectedHi
          .toString(16)
          .padStart(2, "0")} runtime=0x${runtimeHi.toString(16).padStart(2, "0")}`
      );
    }
  }

  const failures = [];
  if (missingCells.length > 0) {
    failures.push(
      `runtime memory missing ${missingCells.length} materialized cell(s): ${missingCells
        .slice(0, 8)
        .join(", ")}${missingCells.length > 8 ? ", ..." : ""}`
    );
  }
  if (mismatches.length > 0) {
    failures.push(...mismatches.slice(0, 32));
    if (mismatches.length > 32) {
      failures.push(`... ${mismatches.length - 32} additional byte mismatch(es) omitted`);
    }
  }
  return failures;
}

function classifyCaseFailure(probeResult, failures) {
  if (!Array.isArray(failures) || failures.length === 0) return "pass";
  if (probeResult && (probeResult.ok === false || probeResult.error)) return "error";
  if (probeResult && probeResult.timeout) return "timeout";
  return "fail";
}

function classifyThrownFailure(err) {
  if (err && typeof err === "object" && err.kind === "timeout") return "timeout";
  const message = err instanceof Error ? err.message : String(err);
  return /timed out|ETIMEDOUT/i.test(message) ? "timeout" : "error";
}

function resolveCases(rawCases, selectedIds) {
  let cases = rawCases;
  if (selectedIds.size > 0) {
    cases = rawCases.filter((testCase) => selectedIds.has(testCase.id));
    const missing = [...selectedIds].filter(
      (id) => !rawCases.some((testCase) => testCase.id === id)
    );
    if (missing.length > 0) {
      throw new Error(`unknown case id(s): ${missing.join(", ")}`);
    }
  }
  return cases;
}

function validateCases(rawCases) {
  if (!Array.isArray(rawCases)) {
    throw new Error("cases.json must contain an array");
  }
  const seenIds = new Set();
  for (const [index, testCase] of rawCases.entries()) {
    const at = `cases[${index}]`;
    if (!testCase || typeof testCase !== "object") {
      throw new Error(`${at} must be an object`);
    }
    if (typeof testCase.id !== "string" || testCase.id.trim() === "") {
      throw new Error(`${at}.id must be a non-empty string`);
    }
    if (seenIds.has(testCase.id)) {
      throw new Error(`duplicate case id "${testCase.id}"`);
    }
    seenIds.add(testCase.id);
    if (typeof testCase.source !== "string" || testCase.source.trim() === "") {
      throw new Error(`${at}.source must be a non-empty string`);
    }
    if (!testCase.expect || typeof testCase.expect !== "object") {
      throw new Error(`${at}.expect must be an object`);
    }
    parseCaseInputBytes(testCase.console_input, `${at}.console_input`);
  }
  return rawCases;
}

function hasCaseTag(testCase, tag) {
  return Array.isArray(testCase.tags) && testCase.tags.includes(tag);
}

function isLengthyCase(testCase) {
  return hasCaseTag(testCase, "lengthy");
}

function isBrokenCase(testCase) {
  return hasCaseTag(testCase, "broken");
}

function isUnimplementedCase(testCase) {
  return hasCaseTag(testCase, "unimplemented");
}

function parseCliArgs(argv) {
  const ids = new Set();
  let includeLengthy = INCLUDE_LENGTHY_BY_DEFAULT;
  let onlyLengthy = false;
  let includeBroken = INCLUDE_BROKEN_BY_DEFAULT;
  let onlyBroken = false;
  let retries = CASE_RETRIES_BY_DEFAULT;
  let jobs = CASE_JOBS_BY_DEFAULT;
  let caseNodeHeapMb = CASE_NODE_HEAP_MB_BY_DEFAULT;
  let jsonSummary = false;
  let dumpMemoryFirst = true;
  let dumpMemoryOnly = false;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--include-lengthy") {
      includeLengthy = true;
      continue;
    }
    if (arg === "--only-lengthy") {
      includeLengthy = true;
      onlyLengthy = true;
      continue;
    }
    if (arg === "--include-broken") {
      includeBroken = true;
      continue;
    }
    if (arg === "--only-broken") {
      includeBroken = true;
      onlyBroken = true;
      continue;
    }
    if (arg === "--retries") {
      const value = argv[i + 1];
      if (value === undefined) {
        throw new Error("--retries requires a value");
      }
      retries = parseRequiredPositiveInt(value, "--retries");
      i += 1;
      continue;
    }
    if (arg.startsWith("--retries=")) {
      retries = parseRequiredPositiveInt(arg.slice("--retries=".length), "--retries");
      continue;
    }
    if (arg === "--jobs") {
      const value = argv[i + 1];
      if (value === undefined) {
        throw new Error("--jobs requires a value");
      }
      jobs = parseRequiredPositiveInt(value, "--jobs");
      i += 1;
      continue;
    }
    if (arg.startsWith("--jobs=")) {
      jobs = parseRequiredPositiveInt(arg.slice("--jobs=".length), "--jobs");
      continue;
    }
    if (arg === "--case-node-heap-mb") {
      const value = argv[i + 1];
      if (value === undefined) {
        throw new Error("--case-node-heap-mb requires a value");
      }
      caseNodeHeapMb = parseRequiredPositiveInt(value, "--case-node-heap-mb");
      i += 1;
      continue;
    }
    if (arg.startsWith("--case-node-heap-mb=")) {
      caseNodeHeapMb = parseRequiredPositiveInt(
        arg.slice("--case-node-heap-mb=".length),
        "--case-node-heap-mb"
      );
      continue;
    }
    if (arg === "--json-summary") {
      jsonSummary = true;
      continue;
    }
    if (arg === "--dump-memory-first") {
      dumpMemoryFirst = true;
      continue;
    }
    if (arg === "--no-dump-memory") {
      dumpMemoryFirst = false;
      dumpMemoryOnly = false;
      continue;
    }
    if (arg === "--dump-memory-only") {
      dumpMemoryFirst = true;
      dumpMemoryOnly = true;
      continue;
    }
    if (arg.startsWith("--")) {
      throw new Error(`unknown flag: ${arg}`);
    }
    ids.add(arg);
  }

  return {
    ids,
    includeLengthy,
    onlyLengthy,
    includeBroken,
    onlyBroken,
    retries,
    jobs,
    caseNodeHeapMb,
    jsonSummary,
    dumpMemoryFirst,
    dumpMemoryOnly,
  };
}

function ensureChromePath(bin) {
  if (!bin.includes(path.sep)) return;
  if (!fs.existsSync(bin)) {
    throw new Error(
      `Chrome binary not found at ${bin}. Set WSS_CHROME to your Chrome/Chromium path.`
    );
  }
}

function ensureWssBinary() {
  if (fs.existsSync(WSS_BIN)) return;
  runChecked(
    "cargo",
    ["build", "--release", "--quiet", "--bin", "wss"],
    ROOT,
    "[setup] build wss",
    WSS_BUILD_TIMEOUT_MS
  );
  if (!fs.existsSync(WSS_BIN)) {
    throw new Error(`wss binary not found at ${WSS_BIN}`);
  }
}

function buildCaseConfig(testCase) {
  const jsCoprocessor = parseBool(
    testCase.js_coprocessor,
    DEFAULT_WSS_JS_COPROCESSOR
  );
  const requestedJsClock = parseBool(testCase.js_clock, DEFAULT_WSS_JS_CLOCK);
  // The runtime requires js_clock when js_coprocessor is enabled.
  const jsClock = requestedJsClock || jsCoprocessor;

  return {
    maxFrames: parsePositiveInt(testCase.max_frames, MAX_FRAMES),
    inputBytes: parseCaseInputBytes(
      testCase.console_input,
      `case "${testCase.id}".console_input`
    ),
    referenceTypes: parseBool(testCase.reference_types, false),
    wasmStackSize: parsePositiveInt(testCase.wasm_stack_size, DEFAULT_WASM_STACK_SIZE),
    memoryBytes: parsePositiveInt(testCase.memory_bytes, DEFAULT_WSS_MEMORY_BYTES),
    stackSlots: parsePositiveInt(testCase.stack_slots, DEFAULT_WSS_STACK_SLOTS),
    jsClock,
    jsCoprocessor,
    virtualTimeBudgetMs: parsePositiveInt(
      testCase.virtual_time_budget_ms,
      VIRTUAL_TIME_BUDGET_MS
    ),
  };
}

function ensureCaseSourceExists(artifacts) {
  if (!fs.existsSync(artifacts.sourcePath)) {
    throw new Error(`missing case source: ${artifacts.sourcePath}`);
  }
}

function compileCaseToWasm(testCase, artifacts, caseConfig) {
  runChecked(
    "clang",
    buildClangArgs(artifacts.sourcePath, artifacts.wasmPath, caseConfig),
    ROOT,
    `[${testCase.id}] clang compile`,
    CLANG_TIMEOUT_MS
  );
}

function transpileCaseToHtml(testCase, artifacts, caseConfig) {
  runChecked(
    WSS_BIN,
    buildWssArgs(artifacts.wasmPath, artifacts.htmlPath, caseConfig),
    ROOT,
    `[${testCase.id}] wss run`,
    WSS_TIMEOUT_MS
  );
}

function makeWasmImportFunction(moduleName, fieldName, inputQueue, outputBytes) {
  if (fieldName === "getchar") {
    return () => (inputQueue.length > 0 ? inputQueue.shift() : -1);
  }
  if (fieldName === "putchar") {
    return (value) => {
      outputBytes.push(Number(value) & 0xff);
      return Number(value) | 0;
    };
  }
  if (fieldName === "clock_ms") {
    return () => 0;
  }
  return (...args) => (args.length > 0 ? Number(args[0]) | 0 : 0);
}

function buildWasmImportObject(module, testCase, caseConfig) {
  const imports = WebAssembly.Module.imports(module);
  const inputQueue = caseConfig.inputBytes.slice();
  const outputBytes = [];
  const namespaces = new Map();

  for (const entry of imports) {
    let namespace = namespaces.get(entry.module);
    if (!namespace) {
      namespace = {};
      namespaces.set(entry.module, namespace);
    }

    if (entry.kind === "function") {
      namespace[entry.name] = makeWasmImportFunction(
        entry.module,
        entry.name,
        inputQueue,
        outputBytes
      );
      continue;
    }

    throw new Error(
      `[${testCase.id}] unsupported wasm import kind "${entry.kind}" for ${entry.module}.${entry.name}`
    );
  }

  return {
    imports: Object.fromEntries(namespaces),
    outputBytes,
  };
}

function dumpCaseMemory(testCase, artifacts, caseConfig) {
  const wasmBytes = fs.readFileSync(artifacts.wasmPath);
  const module = new WebAssembly.Module(wasmBytes);
  const { imports, outputBytes } = buildWasmImportObject(module, testCase, caseConfig);
  const instance = new WebAssembly.Instance(module, imports);
  const exportedMemory = instance.exports.memory;
  if (!(exportedMemory instanceof WebAssembly.Memory)) {
    throw new Error(`[${testCase.id}] expected exported memory`);
  }

  const meta = {
    id: testCase.id,
    source: testCase.source,
    memoryDumpPath: path.basename(artifacts.memoryDumpPath),
    memoryMetaPath: path.basename(artifacts.memoryMetaPath),
    byteLength: 0,
    pages: 0,
    ra: null,
    trap: null,
    console_input_bytes: caseConfig.inputBytes,
    console_output_bytes: outputBytes,
    imports: WebAssembly.Module.imports(module),
    exports: WebAssembly.Module.exports(module),
  };

  try {
    if (typeof instance.exports._start !== "function") {
      throw new Error("missing exported _start");
    }
    meta.ra = formatHex32(instance.exports._start());
  } catch (err) {
    meta.trap = err instanceof Error ? err.message : String(err);
  }

  const finalMemory = Buffer.from(new Uint8Array(exportedMemory.buffer));
  meta.byteLength = finalMemory.length;
  meta.pages = finalMemory.length / 65536;

  fs.writeFileSync(artifacts.memoryDumpPath, finalMemory);
  fs.writeFileSync(artifacts.memoryMetaPath, `${JSON.stringify(meta, null, 2)}\n`);
}

function dumpMemoryForCases(cases) {
  if (cases.length === 0) return new Set();

  const dumpedIds = new Set();
  for (const testCase of cases) {
    const artifacts = getCaseArtifacts(testCase);
    ensureCaseSourceExists(artifacts);
    const caseConfig = buildCaseConfig(testCase);
    compileCaseToWasm(testCase, artifacts, caseConfig);
    dumpCaseMemory(testCase, artifacts, caseConfig);
    dumpedIds.add(testCase.id);
  }

  return dumpedIds;
}

function buildProbeBudget(testCase, virtualTimeBudgetMs) {
  const lengthy = isLengthyCase(testCase);
  const normalBudget = Math.min(virtualTimeBudgetMs, NORMAL_TIMEOUT_MS);
  if (lengthy) {
    return {
      budget: virtualTimeBudgetMs,
      timeout: virtualTimeBudgetMs + DUMP_TIMEOUT_BUFFER_MS,
    };
  }
  const firstBufferMs = Math.min(2000, DUMP_TIMEOUT_BUFFER_MS);
  return { budget: normalBudget, timeout: normalBudget + firstBufferMs };
}

function makeChromeProfilePrefix(testCaseId) {
  return path.join(OUT_DIR, `${testCaseId}.chrome-profile-`);
}

function writeDomAndParseProbe(domPath, domText) {
  fs.writeFileSync(domPath, domText);
  return parseProbeResult(domText);
}

function runChromeProbe(testCase, probePath, domPath, probeBudget) {
  const chromeProfileDir = fs.mkdtempSync(makeChromeProfilePrefix(testCase.id));
  try {
    const chromeArgs = [
      "--headless=new",
      "--disable-gpu",
      "--allow-file-access-from-files",
      "--disable-background-networking",
      "--disable-dev-shm-usage",
      "--no-first-run",
      "--no-default-browser-check",
      "--noerrdialogs",
      "--test-type",
      "--disable-session-crashed-bubble",
      `--user-data-dir=${chromeProfileDir}`,
      `--virtual-time-budget=${probeBudget.budget}`,
      "--dump-dom",
      `file://${probePath}`,
    ];
    try {
      const chrome = runChecked(
        CHROME_BIN,
        chromeArgs,
        ROOT,
        `[${testCase.id}] chrome dump-dom`,
        probeBudget.timeout
      );
      return writeDomAndParseProbe(domPath, chrome.stdout || "");
    } catch (err) {
      const timedOutAfterDump =
        err &&
        err.kind === "timeout" &&
        typeof err.stdout === "string" &&
        err.stdout.includes('id="wss-test-result"');
      if (!timedOutAfterDump) throw err;
      return writeDomAndParseProbe(domPath, err.stdout);
    }
  } finally {
    fs.rmSync(chromeProfileDir, { recursive: true, force: true });
  }
}

function runCase(testCase, options = {}) {
  const artifacts = getCaseArtifacts(testCase);
  ensureCaseSourceExists(artifacts);
  const caseConfig = buildCaseConfig(testCase);
  if (!options.skipCompile) {
    compileCaseToWasm(testCase, artifacts, caseConfig);
  }
  transpileCaseToHtml(testCase, artifacts, caseConfig);
  const html = fs.readFileSync(artifacts.htmlPath, "utf8");
  const materializedMemoryKeys = inferMaterializedMemoryKeys(html);

  injectProbeHtml(
    artifacts.htmlPath,
    artifacts.probePath,
    materializedMemoryKeys,
    caseConfig.maxFrames,
    caseConfig.inputBytes
  );
  const probeBudget = buildProbeBudget(testCase, caseConfig.virtualTimeBudgetMs);
  const probeResult = runChromeProbe(
    testCase,
    artifacts.probePath,
    artifacts.domPath,
    probeBudget
  );

  const failures = validateCase(testCase, probeResult);
  if (options.compareDumpBytes) {
    failures.push(
      ...compareDumpWithMaterializedRuntime(
        testCase,
        artifacts,
        materializedMemoryKeys,
        probeResult
      )
    );
  }
  const kind = classifyCaseFailure(probeResult, failures);

  return { probeResult, failures, domPath: artifacts.domPath, kind };
}

function evaluateCaseWithRetries(testCase, retries, options = {}) {
  const startedAt = Date.now();
  let passed = false;
  let attemptsUsed = 0;
  /** @type {{
   *   kind: "fail" | "timeout" | "error",
   *   failures: string[],
   *   result?: unknown,
   *   domPath?: string
   * } | null} */
  let finalFailure = null;
  for (let attempt = 1; attempt <= retries; attempt += 1) {
    attemptsUsed = attempt;
    try {
      const outcome = runCase(testCase, {
        compareDumpBytes: options.compareDumpBytes,
        skipCompile: options.skipCompile,
      });
      if (outcome.kind === "pass") {
        passed = true;
        break;
      }
      finalFailure = {
        kind: outcome.kind,
        failures: outcome.failures,
        result: outcome.probeResult,
        domPath: outcome.domPath,
      };
    } catch (err) {
      const kind = classifyThrownFailure(err);
      finalFailure = {
        kind,
        failures: [err instanceof Error ? err.message : String(err)],
      };
    }
  }

  const elapsedMs = Date.now() - startedAt;
  if (passed) {
    return {
      id: testCase.id,
      kind: "pass",
      elapsedMs,
      attemptsUsed,
      retries,
      failures: [],
    };
  }

  if (!finalFailure) {
    finalFailure = {
      kind: "error",
      failures: ["internal error: missing failure details after retries"],
    };
  }
  return {
    id: testCase.id,
    kind: finalFailure.kind,
    elapsedMs,
    attemptsUsed,
    retries,
    failures: finalFailure.failures,
    result: finalFailure.result,
    domPath: finalFailure.domPath,
  };
}

function nodeOptionsWithHeapLimit(existingNodeOptions, heapMb) {
  const heapFlagPrefix = "--max-old-space-size=";
  const existing = String(existingNodeOptions || "").trim();
  if (
    existing
      .split(/\s+/)
      .filter(Boolean)
      .some((token) => token.startsWith(heapFlagPrefix))
  ) {
    return existing;
  }
  const heapFlag = `${heapFlagPrefix}${heapMb}`;
  return existing ? `${existing} ${heapFlag}` : heapFlag;
}

function runCaseInSubprocess(testCaseId, retries, caseNodeHeapMb) {
  return new Promise((resolve) => {
    const scriptPath = path.join(TEST_DIR, "run_blackbox.js");
    const args = [scriptPath, "--json-summary", "--jobs=1", `--retries=${retries}`, testCaseId];
    const env = {
      ...process.env,
      NODE_OPTIONS: nodeOptionsWithHeapLimit(process.env.NODE_OPTIONS, caseNodeHeapMb),
    };
    const child = spawn(process.execPath, args, {
      cwd: ROOT,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += String(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderr += String(chunk);
    });
    child.on("error", (err) => {
      resolve({
        id: testCaseId,
        kind: "error",
        elapsedMs: 0,
        attemptsUsed: retries,
        retries,
        failures: [`worker spawn error: ${err instanceof Error ? err.message : String(err)}`],
      });
    });
    child.on("close", () => {
      const lines = stdout
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean);
      let parsed = null;
      for (let i = lines.length - 1; i >= 0; i -= 1) {
        try {
          parsed = JSON.parse(lines[i]);
          break;
        } catch {
          continue;
        }
      }
      if (!parsed || typeof parsed !== "object") {
        const extra = [stdout.trim(), stderr.trim()].filter(Boolean).join("\n\n");
        resolve({
          id: testCaseId,
          kind: "error",
          elapsedMs: 0,
          attemptsUsed: retries,
          retries,
          failures: [
            `worker case did not return JSON summary for ${testCaseId}`,
            ...(extra ? [extra] : []),
          ],
        });
        return;
      }
      resolve(parsed);
    });
  });
}

async function runCasesInParallel(cases, retries, jobs, caseNodeHeapMb) {
  const outcomes = [];
  const queue = cases.slice();
  const totalCases = queue.length;
  const maxJobs = Math.max(1, Math.min(jobs, queue.length));
  const inflight = new Set();
  let completedCases = 0;

  const scheduleNext = () => {
    if (queue.length === 0) return;
    const testCase = queue.shift();
    if (!testCase) return;
    const task = runCaseInSubprocess(testCase.id, retries, caseNodeHeapMb)
      .then((outcome) => {
        outcomes.push(outcome);
        completedCases += 1;
        const rendered = renderOutcomeLabel(outcome);
        console.log(`- ${formatProgressCounter(completedCases, totalCases)} ${testCase.id} ... ${rendered}`);
      })
      .finally(() => {
        inflight.delete(task);
      });
    inflight.add(task);
  };

  for (let i = 0; i < maxJobs; i += 1) {
    scheduleNext();
  }

  while (inflight.size > 0) {
    await Promise.race(inflight);
    scheduleNext();
  }

  return outcomes;
}

function renderOutcomeLabel(outcome) {
  const retryInfo =
    outcome.attemptsUsed > 1 ? `, attempt ${outcome.attemptsUsed}/${outcome.retries}` : "";
  if (outcome.kind === "pass") {
    return `PASS (${outcome.elapsedMs}ms${retryInfo})`;
  }
  const label =
    outcome.kind === "timeout" ? "TIMEOUT" : outcome.kind === "error" ? "ERROR" : "FAIL";
  const suffix = outcome.retries > 1 ? ` after ${outcome.retries} attempt(s)` : "";
  return `${label}${suffix} (${outcome.elapsedMs}ms)`;
}

function formatProgressCounter(current, total) {
  const width = Math.max(2, String(total).length);
  return `[${String(current).padStart(width, "0")}/${String(total).padStart(width, "0")}]`;
}

function writeCasePrefix(testCaseId, current, total) {
  process.stdout.write(`- ${formatProgressCounter(current, total)} ${testCaseId} ... `);
}

function reportSummary(totalCases, outcomes) {
  const counts = {
    pass: 0,
    fail: 0,
    timeout: 0,
    error: 0,
  };
  const failures = [];
  for (const outcome of outcomes) {
    counts[outcome.kind] += 1;
    if (outcome.kind !== "pass") {
      failures.push(outcome);
    }
  }

  console.log(
    `\nSummary: ${counts.pass}/${totalCases} passed, ${counts.fail} failed, ${counts.timeout} timed out, ${counts.error} errors`
  );

  if (failures.length === 0) return;

  for (const failure of failures) {
    const kindLabel = String(failure.kind || "fail").toUpperCase();
    console.log(`\n[${failure.id}] ${kindLabel}`);
    for (const item of failure.failures || []) {
      console.log(`  - ${item}`);
    }
    if (failure.result) {
      console.log(
        `  - probe: pc=${failure.result.pc}, timeout=${failure.result.timeout}, rendered=${JSON.stringify(
          failure.result.rendered_normalized
        )}`
      );
    }
    if (failure.domPath) {
      console.log(`  - dom dump: ${failure.domPath}`);
    }
  }
  console.log("\nUnsuccessful tests:");
  for (const failure of failures) {
    const kindLabel = String(failure.kind || "fail").toUpperCase();
    console.log(`  - ${failure.id} (${kindLabel})`);
  }
  const failedIds = failures.map((failure) => failure.id);
  console.log(`\nRerun unsuccessful only: node tests/run_blackbox.js ${failedIds.join(" ")}`);
  process.exitCode = 1;
}

async function main() {
  ensureChromePath(CHROME_BIN);
  ensureWssBinary();
  fs.mkdirSync(OUT_DIR, { recursive: true });

  const rawCases = validateCases(JSON.parse(fs.readFileSync(CASES_PATH, "utf8")));
  const cli = parseCliArgs(process.argv.slice(2));
  let cases = resolveCases(rawCases, cli.ids);
  let skippedLengthy = 0;
  let skippedBroken = 0;
  let skippedUnimplemented = 0;
  if (cli.onlyLengthy) {
    cases = cases.filter(isLengthyCase);
  } else if (cli.ids.size === 0 && !cli.includeLengthy) {
    skippedLengthy = cases.filter(isLengthyCase).length;
    cases = cases.filter((testCase) => !isLengthyCase(testCase));
  }
  if (cli.onlyBroken) {
    cases = cases.filter(isBrokenCase);
  } else if (cli.ids.size === 0 && !cli.includeBroken) {
    skippedBroken = cases.filter(isBrokenCase).length;
    cases = cases.filter((testCase) => !isBrokenCase(testCase));
  }
  if (cli.ids.size === 0) {
    skippedUnimplemented = cases.filter(isUnimplementedCase).length;
    cases = cases.filter((testCase) => !isUnimplementedCase(testCase));
  }
  if (cases.length === 0) {
    throw new Error("no cases selected");
  }

  const runInParallel = cli.jobs > 1 && cases.length > 1;
  const preDumpedCaseIds =
    cli.dumpMemoryFirst && (!runInParallel || cli.dumpMemoryOnly)
      ? dumpMemoryForCases(cases)
      : new Set();

  if (cli.dumpMemoryOnly) {
    return;
  }

  if (cli.jsonSummary) {
    if (cases.length !== 1) {
      throw new Error("--json-summary requires exactly one selected case");
    }
    const outcome = evaluateCaseWithRetries(cases[0], cli.retries, {
      compareDumpBytes: preDumpedCaseIds.has(cases[0].id),
      skipCompile: preDumpedCaseIds.has(cases[0].id),
    });
    console.log(JSON.stringify(outcome));
    if (outcome.kind !== "pass") {
      process.exitCode = 1;
    }
    return;
  }

  console.log(`Running ${cases.length} blackbox case(s)...`);
  if (cli.retries > 1) {
    console.log(`Retries per case: ${cli.retries}`);
  }
  if (cli.jobs > 1 && cases.length > 1) {
    console.log(
      `Parallel jobs: ${Math.min(cli.jobs, cases.length)} (heap limit ${cli.caseNodeHeapMb}MB per case)`
    );
  }
  if (skippedLengthy > 0) {
    console.log(
      `Skipping ${skippedLengthy} lengthy case(s). Use --include-lengthy to run them.`
    );
  }
  if (skippedBroken > 0) {
    console.log(
      `Skipping ${skippedBroken} broken case(s). Use --include-broken to run them.`
    );
  }
  if (skippedUnimplemented > 0) {
    console.log(
      `Skipping ${skippedUnimplemented} unimplemented case(s). Select by case id to run them.`
    );
  }

  /** @type {any[]} */
  let outcomes = [];
  if (runInParallel) {
    outcomes = await runCasesInParallel(cases, cli.retries, cli.jobs, cli.caseNodeHeapMb);
  } else {
    for (const [index, testCase] of cases.entries()) {
      writeCasePrefix(testCase.id, index + 1, cases.length);
      const outcome = evaluateCaseWithRetries(testCase, cli.retries, {
        compareDumpBytes: preDumpedCaseIds.has(testCase.id),
        skipCompile: preDumpedCaseIds.has(testCase.id),
      });
      console.log(renderOutcomeLabel(outcome));
      outcomes.push(outcome);
    }
  }

  reportSummary(cases.length, outcomes);
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : String(err));
  process.exitCode = 1;
});
