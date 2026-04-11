<script id="wss-test-probe">
(() => {
  const terminalPcs = new Set([-1, -2, -3, -4, -5]);
  const maxFrames = __WSS_MAX_FRAMES__;
  const memoryKeys = __WSS_MEMORY_KEYS__;
  const inputQueue = __WSS_INPUT_BYTES__;
  const getcharPcs = new Set(__WSS_GETCHAR_PCS__);
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
</script>
