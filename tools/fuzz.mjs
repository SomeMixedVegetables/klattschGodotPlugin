// Seeded differential fuzzer: generates random inputs for every feature, runs
// them through the original JS engine, and records the outputs for the Rust
// parity tests (crates/klattsch-core/tests/fuzz_parity.rs).
//
//   node tools/fuzz.mjs [--seed N] [--scale K] [path/to/klattsch-main]
//
// --scale multiplies the case counts (default 1). Rendered audio and WAV bytes
// are stored as hashes; non-finite numbers are tagged as {"$f": "NaN"} etc.
// because JSON cannot represent them.

import { mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { registerHooks } from 'node:module';
import { loadWorklet } from './worklet-harness.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const argv = process.argv.slice(2);
const flag = (name, dflt) => {
  const i = argv.indexOf(name);
  if (i === -1) return dflt;
  const v = argv[i + 1];
  argv.splice(i, 2);
  return Number(v);
};
const SEED = flag('--seed', 1);
const SCALE = flag('--scale', 1);
const jsRoot = resolve(argv[0] ?? join(root, '..', 'klattsch-main'));
const outDir = join(root, 'fixtures', 'fuzz');
const dictPath = join(root, 'data', 'cmudict', 'cmudict-0.7b');
const n = (k) => Math.max(1, Math.round(k * SCALE));

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

const shimUrl = pathToFileURL(join(here, 'cmu-shim.mjs')).href;
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === 'cmu-pronouncing-dictionary') return { url: shimUrl, shortCircuit: true };
    return next(specifier, context);
  },
});
const engineUrl = (p) => pathToFileURL(join(jsRoot, 'src', ...p.split('/'))).href;
const E = await import(engineUrl('engine/index.js'));
const D = await import(engineUrl('engine/dsp.js'));
const K = await import(engineUrl('engine/kana.js'));
const P = await import(engineUrl('engine/pronounce.js'));
const shim = await import(shimUrl);
globalThis.document = {
  createDocumentFragment: () => ({ nodes: [], appendChild(x) { this.nodes.push(x); } }),
  createElement: () => ({ className: '', textContent: '' }),
  createTextNode: (t) => ({ className: null, textContent: t }),
};
const H = await import(engineUrl('highlight.js'));

// --- helpers ------------------------------------------------------------------
// mulberry32
function makeRng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6D2B79F5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
let R = makeRng(SEED);
const chance = (p) => R() < p;
const int = (lo, hi) => lo + Math.floor(R() * (hi - lo + 1));
const pick = (arr) => arr[Math.floor(R() * arr.length)];
const weighted = (pairs) => {
  const total = pairs.reduce((s, [w]) => s + w, 0);
  let x = R() * total;
  for (const [w, f] of pairs) { if ((x -= w) < 0) return f(); }
  return pairs[pairs.length - 1][1]();
};

// JSON with non-finite numbers tagged.
const tag = (v) => {
  if (typeof v === 'number' && !Number.isFinite(v)) return { $f: String(v) };
  if (Array.isArray(v)) return v.map(tag);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, tag(x)]));
  return v;
};
const writeJson = (name, data) => writeFileSync(join(outDir, name), JSON.stringify(tag(data)));

// Two 32-bit lanes over the f32 bit patterns; NaNs are canonicalized because
// their payload bits are not specified by either language.
function hashF32(f32) {
  const u = new Uint32Array(f32.buffer, f32.byteOffset, f32.length);
  let h1 = 0x811c9dc5 | 0, h2 = 0x9747b28c | 0;
  for (let i = 0; i < u.length; i++) {
    let w = u[i];
    if ((w & 0x7f800000) === 0x7f800000 && (w & 0x007fffff) !== 0) w = 0x7fc00000;
    h1 = Math.imul(h1 ^ w, 0x01000193);
    h2 = Math.imul(h2 ^ ((w >>> 16) | (w << 16)), 0x5bd1e995) ^ (h2 >>> 15);
  }
  return `${(h1 >>> 0).toString(16)}:${(h2 >>> 0).toString(16)}:${u.length}`;
}
function hashBytes(bytes) {
  let h1 = 0x811c9dc5 | 0, h2 = 0x9747b28c | 0;
  for (let i = 0; i < bytes.length; i++) {
    const w = bytes[i];
    h1 = Math.imul(h1 ^ w, 0x01000193);
    h2 = Math.imul(h2 ^ (w << 8), 0x5bd1e995) ^ (h2 >>> 15);
  }
  return `${(h1 >>> 0).toString(16)}:${(h2 >>> 0).toString(16)}:${bytes.length}`;
}
const errMsg = (fn) => { try { fn(); return null; } catch (e) { return String(e.message); } };

// Words the JS treats as Object.prototype lookups (a known JS bug; see README).
const PROTO_WORDS = new Set(Object.getOwnPropertyNames(Object.prototype));

// --- random numbers in source text ------------------------------------------------
const numText = () => weighted([
  [5, () => String(int(0, 400))],
  [3, () => `${int(0, 50)}.${int(0, 999)}`],
  [1, () => `0${int(0, 9)}`],
  [1, () => String(int(0, 9)).repeat(int(10, 25))],
  [1, () => String(int(1, 9)).repeat(int(309, 400))], // overflows to Infinity
  [1, () => `${int(0, 3)}.${'0'.repeat(int(0, 5))}${int(1, 9)}`],
]);
const signed = () => (chance(0.5) ? '+' : '-') + numText();

// --- random banks ---------------------------------------------------------------
const BUNDLED = E.banks.list();
const allCodes = new Set();
for (const b of BUNDLED) for (const k of Object.keys(E.banks.get(b).phonemes)) allCodes.add(k);
const CODE_POOL = [...allCodes, 'QX', 'QY', 'ZZZ', 'X', 'AAA', 'OO'];
const fuzzBankNames = [];

// Synth parameters must be numbers, booleans or null: the JS synth keeps a
// string target as-is and later concatenates onto it (see README).
const synthNum = () => weighted([
  [8, () => Math.round(R() * 1000) / 1000], [2, () => int(0, 3000)], [1, () => R() < 0.5],
  [1, () => null], [1, () => -R()], [1, () => 0],
]);
const scaledField = () => weighted([
  [24, () => int(100, 5000)], [3, () => String(int(100, 4000))], [3, () => null], [3, () => true],
  [4, () => pick([' 300 ', '3e2', '0x1F', '0X1f', '0b11', '0o7', 'abc', '', 'Infinity', '-Infinity', '+5', '.5', '5.', '1_0', '0x', '-0x5', '1e', 'inf', 'NaN', '0x1G', '0b12', '2e+2', '5e-1', '-1E-1', '\u00A0400\u000A'])],
  [3, () => pick([[300], [], [1, 2], {}, ['250'], [null], [[7]]])],
]);
function randomPhoneme() {
  const p = {};
  const keys = ['voicing', 'F1', 'F2', 'F3', 'BW1', 'BW2', 'BW3', 'A1', 'A2', 'A3'];
  for (const k of keys) {
    if (chance(0.92)) p[k] = /^(F|BW)/.test(k) ? scaledField() : synthNum();
  }
  if (chance(0.15)) p.isStop = pick([true, false, 1, 0, 'yes', '', null]);
  if (chance(0.2)) {
    p.glideTo = weighted([
      [6, () => { const g = {}; for (const k of ['F1', 'F2', 'F3', 'BW1', 'F4']) if (chance(0.6)) g[k] = scaledField(); return g; }],
      [1, () => true], [1, () => 0], [1, () => ({})], [1, () => null], [1, () => 'ab'], [1, () => [400, 900]],
    ]);
  }
  for (const k of ['F4', 'BW4', 'F5', 'F9', 'BW9', 'F10', 'BW3x']) if (chance(0.08)) p[k] = scaledField();
  if (chance(0.1)) p.gain = synthNum();
  if (chance(0.1)) p.effort = synthNum();
  if (chance(0.1)) p.vibratoDepth = synthNum();
  if (chance(0.2)) p.ipa = pick(['a', 'ä', 'e̞', '\u{1F600}', '']);
  if (chance(0.1)) p.example = pick(['[a]ru', 'x', 'ü']);
  return p;
}
function randomBankEntries() {
  const ph = {};
  const count = int(0, 8);
  for (let i = 0; i < count; i++) {
    ph[pick(CODE_POOL)] = weighted([[6, randomPhoneme], [2, () => null], [1, () => 0], [1, () => 'str'], [1, () => [1, 'x']], [1, () => []], [1, () => 5], [1, () => true]]);
  }
  return ph;
}

const bankScenarios = [];
for (let s = 0; s < n(120); s++) {
  const names = Array.from({ length: int(1, 5) }, (_, i) => `fz${s}-${i}`);
  const ops = [];
  for (const name of names) {
    const bank = {
      schemaVersion: weighted([[24, () => 1], [2, () => 2], [2, () => '1'], [2, () => undefined],
        [1, () => [1]], [1, () => [1, 2]], [1, () => ({})], [1, () => true], [1, () => null], [1, () => 1.5], [1, () => [null, 'x']]]),
      name: weighted([[14, () => name], [1, () => ''], [1, () => undefined], [1, () => 7]]),
      extends: weighted([
        [3, () => null], [4, () => pick(BUNDLED)], [4, () => pick(names)],
        [1, () => 'missing-bank'], [1, () => undefined], [1, () => ''],
        [1, () => 7], [1, () => true], [1, () => [1]], [1, () => ({})],
        [1, () => (fuzzBankNames.length ? pick(fuzzBankNames) : null)],
      ]),
      phonemes: weighted([[12, randomBankEntries], [1, () => undefined], [1, () => 5]]),
    };
    if (chance(0.3)) bank.displayName = pick([null, 'Display', 3]);
    if (chance(0.3)) bank.language = pick([null, 'xx-YY']);
    if (chance(0.2)) bank.license = 'MIT';
    if (chance(0.2)) bank.source = pick([null, 'src']);
    for (const k of Object.keys(bank)) if (bank[k] === undefined) delete bank[k];
    const error = errMsg(() => E.registerBank(bank));
    ops.push({ bank, error });
    if (!error) fuzzBankNames.push(bank.name);
  }
  const lookups = [...names, pick(BUNDLED), 'missing-bank'].map(name => {
    try {
      const b = E.banks.get(name);
      return { name, resolved: b === undefined ? null : b };
    } catch (e) { return { name, error: String(e.message) }; }
  });
  bankScenarios.push({ ops, lookups, list: E.banks.list() });
}
writeJson('banks.json', bankScenarios);
// Banks that resolve cleanly, for use in phoneme strings.
const usableBanks = [...BUNDLED, ...fuzzBankNames.filter(nm => { try { return !!E.banks.get(nm); } catch { return false; } })];
const resolvedCodes = new Set(CODE_POOL);
for (const nm of usableBanks) for (const k of Object.keys(E.banks.get(nm).phonemes)) resolvedCodes.add(k);
const PHONEME_CODES = [...resolvedCodes].filter(c => /^[A-Z]+$/.test(c));

// --- random phoneme strings --------------------------------------------------------
const NOTE = () => pick(['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H']) + pick(['', '', '#', 'b']) +
  weighted([[8, () => String(int(-1, 9))], [1, () => String(int(10, 400))], [1, () => '-0'], [1, () => '']]);
const BRACKET_KEYS = ['base', 'pitch', 'rate', 'scale', 'vibrato', 'vibratoRate', 'tremolo', 'tremoloRate',
  'aspiration', 'tilt', 'effort', 'pause', 'voice', 'OQ', 'FNZ', 'F1', 'A1', 'BW2', 'F4', 'gain', 'voicing',
  'foo', 'x_1', 'Voice', '_q', 'F', 'Q9'];
const ENGINE_NAMES = ['foo', 'bar.v2', 'x-1_y', 'E'];

function token() {
  return weighted([
    [30, () => {
      let t = pick(PHONEME_CODES);
      if (chance(0.15)) t += pick(["'", '!']);
      if (chance(0.25)) {
        t += weighted([[5, signed], [4, () => `(${signed()})`], [1, () => pick(['(5', '++5', '+', '(+)', '+-3', '(+5', '+5)'])]]);
      }
      return t;
    }],
    [3, () => pick(["'", '!'])],
    [5, () => pick([',', ';', '.'])],
    [5, () => pick(['(', ')'])],
    [14, () => {
      const letter = weighted([[10, () => pick([...'brpsvwmnhtg'])], [1, () => pick([...'aqxz'])]]);
      return letter + weighted([
        [2, () => ''], [4, numText], [3, () => '=' + numText()], [4, signed], [1, () => '=' + signed()],
        [1, () => pick(['=', '1.2.3', '+', '.5'])],
      ]);
    }],
    [5, () => 'b' + (chance(0.3) ? '=' : '') + NOTE()],
    [8, () => `[${pick(BRACKET_KEYS)}=${chance(0.3) ? '-' : ''}${numText()}]`],
    [3, () => `[${pick(BRACKET_KEYS)}]`],
    [1, () => pick(['[OQ=]', '[=1]', '[OQ=1.]', '[OQ=+1]', '[ OQ=1]', '[bank=]', '[voice]'])],
    [4, () => (chance(0.8) ? `[bank=${weighted([[8, () => pick(usableBanks)], [1, () => 'nope'], [1, () => pick(fuzzBankNames.length ? fuzzBankNames : ['x'])]])}]` : '[bank]')],
    [3, () => (chance(0.7) ? `[engine=${pick(ENGINE_NAMES)}]` : '[engine]')],
    [3, () => `[voice=${int(0, 3)}]`],
    [4, () => {
      // junk, homoglyphs, compatibility forms, emoji
      return weighted([
        [2, () => pick(['АА', 'ІҮ', 'ΑΕ', 'Ο', 'Р', 'са', 'ＡＡ', 'Ａ', 'ｂ１４０', 'ﬁ'])],
        [2, () => Array.from({ length: int(1, 5) }, () => pick([...'abcxyz0129+-=()[]#*/\'!.,;_~é'])).join('')],
        [1, () => pick(['\u{1F600}', 'A\u{1F600}', '\u{1F600}\u{1F600}', '\u{10000}'])],
        [1, () => pick(['hello', 'Hello', 'AAbb', 'aa', 'R2D2'])],
      ]);
    }],
  ]);
}
function separator() {
  return weighted([
    [40, () => ' '], [4, () => '  '], [3, () => '\t'], [4, () => '\n'], [1, () => '\r\n'],
    [1, () => ' '], [1, () => '　'], [1, () => ' '], [1, () => ' '], [1, () => '﻿'],
    [1, () => '\u0085'], [1, () => '᠎'], [1, () => '​'],
    [2, () => ' /* c */ '], [1, () => '/**/'], [2, () => ' # note\n'], [1, () => ' #x '],
    [2, () => ''],
  ]);
}
function phonemeString() {
  const len = weighted([[3, () => int(0, 3)], [6, () => int(4, 15)], [2, () => int(16, 40)]]);
  let s = chance(0.1) ? separator() : '';
  for (let i = 0; i < len; i++) {
    let t = token();
    const cps = Array.from(t); // split between code points, never inside a surrogate pair
    if (chance(0.03) && cps.length > 1) {
      const at = int(1, cps.length - 1);
      t = cps.slice(0, at).join('') + pick(['/*x*/', '/**/', '​', '‍']) + cps.slice(at).join('');
    }
    s += t + separator();
  }
  if (chance(0.03)) s += pick(['/* open', '#', '/*', '*/']);
  for (const w of s.split(/\s+/)) if (PROTO_WORDS.has(w)) return phonemeString();
  return s;
}

// --- tokenize / compile / render ------------------------------------------------------
const randomOpts = () => {
  const o = {};
  if (chance(0.3)) o.baseF0 = pick([60, 120, 200.5, 0, 440]);
  if (chance(0.3)) o.rate = pick([40, 80, 110, 250.25, 0]);
  if (chance(0.2)) o.scale = pick([0.8, 1, 1.25]);
  if (chance(0.15)) o.vibratoDepth = pick([0, 3, 12]);
  if (chance(0.15)) o.vibratoRate = pick([0, 5, 7.5]);
  if (chance(0.15)) o.tremoloDepth = pick([0, 0.3, 1]);
  if (chance(0.15)) o.tremoloRate = pick([0, 4]);
  if (chance(0.15)) o.aspiration = pick([0, 0.2, 1]);
  if (chance(0.15)) o.tilt = pick([-0.5, 0, 0.6]);
  if (chance(0.15)) o.effort = pick([0, 0.5, 1]);
  if (chance(0.2)) o.bank = weighted([
    [6, () => pick(usableBanks)],
    [1, () => pick(fuzzBankNames)], // may be broken (cyclic / missing parent)
    [1, () => ({ schemaVersion: 1, name: pick(fuzzBankNames), extends: 'x' })],
    [1, () => ({ schemaVersion: 1, name: `fz-new-${int(0, 1e9)}`, extends: pick(fuzzBankNames) })],
    [1, () => ({ name: 'fz-bad', extends: 'klatt1980-en' })],
    [1, () => ({ name: 'fz-flat', phonemes: randomBankEntries() })],
    [1, () => ({ phonemes: { AA: randomPhoneme(), IY: null } })],
  ]);
  if (chance(0.15)) o.engine = pick(ENGINE_NAMES);
  if (chance(0.15)) o.extras = { OQ: pick([0.5, 0.65]), ...(chance(0.5) ? { FNZ: 300 } : {}) };
  return o;
};
const SAMPLE_RATES = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000, 12345.5, 100];
const MAX_RENDER_SAMPLES = 1_500_000;

const compileCases = [];
for (let i = 0; i < n(4000); i++) {
  const input = phonemeString();
  const opts = chance(0.4) ? randomOpts() : {};
  const tok = E.tokenize(input);
  let result = null, error = null;
  try { result = E.compileString(input, opts); } catch (e) { error = String(e.message); }
  const c = { input, opts, source: tok.source, tokens: tok.tokens, result, error };
  if (result && chance(0.5)) {
    const sr = pick(SAMPLE_RATES);
    c.renders = result.voices.map((v) => {
      if (!v.schedule.length) return null;
      const len = Math.ceil(v.totalMs * sr / 1000);
      if (Number.isFinite(len) && len > MAX_RENDER_SAMPLES) return 'skipped';
      try {
        return hashF32(E.renderToBuffer({ sampleRate: sr, schedule: v.schedule, totalMs: v.totalMs }));
      } catch (e) {
        return { error: String(e.message) };
      }
    });
    c.sr = sr;
  }
  compileCases.push(c);
}
writeJson('compile.json', compileCases);

// --- highlight ----------------------------------------------------------------------
{
  // Like the Rust registry_phonemes_for: broken banks count as unknown.
  const phonemesFor = (name) => {
    try { const b = E.banks.get(name); return b ? new Set(Object.keys(b.phonemes)) : null; } catch { return null; }
  };
  const cases = [];
  for (let i = 0; i < n(3000); i++) {
    const input = phonemeString();
    const mode = pick(['none', 'registry', 'initial']);
    const initialBank = mode === 'initial' ? pick([...usableBanks, 'nope', '']) : null;
    const opts = mode === 'none' ? {} : { phonemesFor, ...(initialBank !== null ? { initialBank } : {}) };
    const nodes = H.buildHighlight(input, opts).nodes.map(x => [x.className, x.textContent]);
    nodes.pop(); // trailing "\n" sentinel (not part of the port)
    cases.push({ input, mode, initialBank, nodes });
  }
  writeJson('highlight.json', cases);
}

// --- live synth ------------------------------------------------------------------------
{
  const PARAMS = E.PARAMS;
  const val = () => weighted([
    [10, () => Math.round(R() * 1000) / 100], [4, () => int(0, 4000)], [2, () => R()], [1, () => -R() * 10],
    [1, () => 0], [1, () => 1e9], [1, () => NaN], [1, () => Infinity], [1, () => -Infinity], [1, () => 1e-320],
  ]);
  const target = () => {
    const t = {};
    for (const k of PARAMS) if (chance(0.3)) t[k] = val();
    if (chance(0.1)) t.notAParam = 5;
    return t;
  };
  const ms = () => weighted([[6, () => int(0, 200)], [2, () => R() * 50], [1, () => -5], [1, () => NaN], [1, () => 1e7], [2, () => undefined]]);
  const events = () => Array.from({ length: int(0, 12) }, () => {
    const e = { target: target() };
    if (chance(0.9)) e.atMs = weighted([[8, () => int(0, 2000)], [1, () => -50], [1, () => R() * 100], [1, () => NaN]]);
    const t = ms();
    if (t !== undefined) e.transitionMs = t;
    return e;
  });
  const scenarios = [];
  for (let s = 0; s < n(400); s++) {
    const sr = pick([...SAMPLE_RATES, 1, 0.5]);
    const ctor = {};
    if (chance(0.5)) ctor.initialTarget = target();
    if (chance(0.5)) ctor.schedule = events();
    const ops = [];
    for (let i = 0; i < int(1, 25); i++) {
      ops.push(weighted([
        [6, () => ({ op: 'process', n: weighted([[5, () => int(0, 2000)], [1, () => int(2000, 20000)], [1, () => 128]]) })],
        [3, () => { const o = { op: 'setTarget', target: target() }; const t = ms(); if (t !== undefined) o.transitionMs = t; return o; }],
        [2, () => { const o = { op: 'queueSchedule', events: events() }; if (chance(0.6)) o.startCounter = weighted([[3, () => int(-5000, 5000)], [1, () => R() * 10], [1, () => NaN]]); return o; }],
        [1, () => (chance(0.5) ? { op: 'reset' } : { op: 'reset', initialTarget: target() })],
      ]));
    }
    const synth = new E.FormantSynth({ sampleRate: sr, ...ctor });
    const chunks = [];
    for (const op of ops) {
      if (op.op === 'process') { const b = new Float32Array(op.n); synth.process(b); chunks.push(b); }
      else if (op.op === 'setTarget') { 'transitionMs' in op ? synth.setTarget(op.target, op.transitionMs) : synth.setTarget(op.target); }
      else if (op.op === 'queueSchedule') { synth.queueSchedule(op.events, op.startCounter); }
      else { synth.reset(op.initialTarget); }
    }
    const all = new Float32Array(chunks.reduce((a, b) => a + b.length, 0));
    let o = 0; for (const c of chunks) { all.set(c, o); o += c.length; }
    scenarios.push({ sr, ctor, ops, hash: hashF32(all) });
  }
  writeJson('synth.json', scenarios);
}

// --- AudioWorklet (src/formant-worklet.js) ------------------------------------
{
  const runWorklet = await loadWorklet(jsRoot);
  const pool = compileCases
    .filter(c => c.result)
    .flatMap(c => c.result.voices.map(v => v.schedule))
    .filter(sch => sch.length && sch.length < 120 && sch.every(e => Number.isFinite(e.atMs) && e.atMs < 4000));
  const val = () => weighted([[12, () => Math.round(R() * 3000) / 10], [3, () => R()], [1, () => 0], [1, () => -R() * 5],
    [1, () => NaN], [1, () => Infinity]]);
  const target = () => {
    const t = {};
    for (const k of E.PARAMS) if (chance(0.25)) t[k] = val();
    if (chance(0.05)) t.extra = 1;
    return t;
  };
  const randomEvents = () => Array.from({ length: int(0, 8) }, () => {
    const e = { target: target() };
    if (chance(0.9)) e.atMs = int(0, 1500);
    if (chance(0.7)) e.transitionMs = weighted([[6, () => int(0, 150)], [1, () => R() * 3]]);
    return e;
  });
  const schedule = () => (chance(0.7) && pool.length ? pick(pool) : randomEvents());
  const voice = (msg) => weighted([
    [3, () => {}], [1, () => { msg.voice = null; }], [6, () => { msg.voice = int(0, 5); }],
  ]);
  const message = () => weighted([
    [4, () => {
      const m = { type: 'frame', target: target() };
      voice(m);
      weighted([[2, () => {}], [1, () => { m.transitionMs = null; }], [4, () => { m.transitionMs = int(0, 300); }],
        [1, () => { m.transitionMs = pick([-10, 0.5, NaN]); }]]);
      return m;
    }],
    [4, () => {
      const m = { type: 'schedule', schedule: schedule() };
      voice(m);
      weighted([[3, () => {}], [1, () => { m.startTime = null; }], [4, () => { m.startTime = R() * 2; }],
        [1, () => { m.startTime = pick([0, -1, NaN, 1e9]); }]]);
      return m;
    }],
    [2, () => {
      const m = { type: 'reset' };
      voice(m);
      weighted([[2, () => {}], [1, () => { m.initialTarget = null; }], [2, () => { m.initialTarget = target(); }]]);
      return m;
    }],
    [1, () => pick([{ type: 'bogus' }, {}, { voice: 1 }])],
  ]);
  const scenarios = [];
  for (let i = 0; i < n(300); i++) {
    const sc = {
      sr: pick([8000, 22050, 44100, 48000, 12345.5]),
      outputs: Array.from({ length: int(0, 4) }, () => weighted([[6, () => 1], [2, () => 2], [1, () => 0]])),
      processorOptions: {},
      ops: [],
    };
    if (chance(0.3)) sc.processorOptions.schedule = schedule();
    if (chance(0.2)) sc.processorOptions.initialTarget = target();
    if (chance(0.4)) {
      sc.processorOptions.schedules = Array.from({ length: int(0, 5) },
        () => weighted([[4, schedule], [1, () => null], [1, () => []]]));
    }
    for (let j = 0; j < int(1, 16); j++) {
      sc.ops.push(chance(0.5) ? { op: 'process', quanta: int(0, 40) } : { op: 'message', data: message() });
    }
    sc.ops.push({ op: 'process', quanta: int(1, 60) });
    const outs = runWorklet(sc);
    scenarios.push({ ...sc, hashes: outs.map(o => (o ? hashF32(o) : null)) });
  }
  writeJson('worklet.json', scenarios);
}

// --- WAV encoder -------------------------------------------------------------------------
{
  const cases = [];
  for (let i = 0; i < n(300); i++) {
    const len = weighted([[1, () => 0], [6, () => int(1, 3000)], [1, () => int(3000, 30000)]]);
    const buf = new Float32Array(len);
    for (let j = 0; j < len; j++) {
      buf[j] = weighted([
        [20, () => (R() * 2 - 1) * pick([1, 0.1, 3])], [1, () => 0], [1, () => -0], [1, () => NaN],
        [1, () => Infinity], [1, () => -Infinity], [1, () => 1e-40], [1, () => 0.5 / 32767], [1, () => -0.5 / 32767],
      ]);
    }
    const opt = {};
    if (chance(0.5)) opt.peakNormalize = pick([0, 0.5, 0.95, 1, 2, -1, 1e-9, NaN, null]);
    if (chance(0.5)) {
      opt.metadata = {};
      if (chance(0.7)) opt.metadata.software = pick(['', 'a', 'ab', 'klattsch ·', 'é\u{1F600}']);
      if (chance(0.7)) opt.metadata.comment = pick(['', 'x', 'HH AH L OW', 'odd!', '日本語']);
    }
    const sampleRate = pick([8000, 22050, 44100, 48000, 96000, 1, 2147483647]);
    const { bytes, gain } = E.encodeWav(buf, sampleRate, opt);
    cases.push({ samples: [...buf].map(x => (Object.is(x, -0) ? '-0' : x)), sampleRate, opt, gain, hash: hashBytes(bytes) });
  }
  writeJson('wav.json', cases);
}

// --- DSP primitives ------------------------------------------------------------------------
{
  const glottal = [];
  for (let i = 0; i < n(20000); i++) {
    const phase = weighted([[8, () => R()], [1, () => R() * 3 - 1], [1, () => pick([0, 1, 0.5, 0.3, 0.38, NaN])]]);
    const effort = weighted([[8, () => R()], [1, () => R() * 4 - 2], [1, () => pick([0, 1, NaN, -Infinity])]]);
    glottal.push([phase, effort, D.glottalPulse(phase, effort)]);
  }
  const soft = [];
  for (let i = 0; i < n(20000); i++) {
    const x = weighted([[8, () => (R() * 2 - 1) * 3], [1, () => (R() * 2 - 1) * 1e6], [1, () => pick([0.85, -0.85, NaN, Infinity, -Infinity, -0])]]);
    soft.push([x, D.softClip(x)]);
  }
  const biquads = [];
  for (let i = 0; i < n(500); i++) {
    const bq = new D.BandpassBiquad();
    const steps = [];
    const seed = int(1, 1e6) | 0;
    let s = seed;
    for (let j = 0; j < int(1, 6); j++) {
      const f = weighted([[6, () => R() * 6000], [1, () => pick([0, -100, 1e6, NaN, 40, 21600])]]);
      const bw = weighted([[6, () => R() * 600], [1, () => pick([0, -5, 1e5, NaN, 20])]]);
      const sr = pick([8000, 22050, 44100, 48000, 96000]);
      bq.setFreq(f, bw, sr);
      const outs = [];
      for (let k = 0; k < 64; k++) { s = D.xorshift(s); outs.push(bq.process(s / 2147483648)); }
      const coeffs = [bq.b0, bq.b1, bq.b2, bq.a1, bq.a2];
      const reset = chance(0.2);
      if (reset) bq.reset();
      steps.push({ f, bw, sr, coeffs, outs, reset });
    }
    biquads.push({ seed, steps });
  }
  const trig = [];
  for (let i = 0; i < n(200000); i++) {
    const x = weighted([
      [6, () => R() * 7 - 0.5], [2, () => (R() - 0.5) * 1e4],
      [2, () => int(-1e6, 1e6) * Math.PI / 2 + (R() - 0.5) * 1e-9],
      [2, () => int(-8, 8) * Math.PI / 2 * (1 + (R() - 0.5) * 2 ** -int(20, 52))],
      [1, () => (R() - 0.5) * 10 ** int(5, 308)], [1, () => (R() - 0.5) * 10 ** -int(5, 320)],
    ]);
    trig.push([x, Math.sin(x), Math.cos(x)]);
  }
  writeJson('dsp.json', { glottal, soft, biquads, trig });
}

// --- kana / romaji ---------------------------------------------------------------------------
{
  const hira = [...'あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんがぎぐげござじずぜぞだぢづでどばびぶべぼぱぴぷぺぽゃゅょっーぃぁゔ'];
  const kata = hira.map(c => { const cp = c.codePointAt(0); return cp >= 0x3041 && cp <= 0x3096 ? String.fromCodePoint(cp + 0x60) : c; });
  const roma = [...'aiueokstnhmyrwgzdbpjfcv', 'sh', 'ch', 'ts', 'ky', 'ny', "n'", 'nn', "'"];
  const misc = [' ', '　', '\t', '-', '1', 'X', 'Ä', '\u{1F600}', '漢', 'ｶ', '。'];
  const cases = [];
  for (let i = 0; i < n(5000); i++) {
    const pools = weighted([[3, () => [hira]], [2, () => [kata]], [3, () => [roma]], [2, () => [hira, kata, roma, misc]]]);
    let s = '';
    for (let j = 0; j < int(0, 14); j++) {
      const pool = pick(pools);
      let piece = pick(pool);
      if (pool === roma && chance(0.1)) piece = piece.toUpperCase();
      s += piece;
    }
    cases.push({ input: s, kana: K.kanaToPhonemes(s), romaji: K.romajiToPhonemes(s), isKana: K.isKana(s), japanese: K.japaneseToPhonemes(s) });
  }
  writeJson('kana.json', cases);
}

// --- pronounce: every dictionary word + random text ---------------------------------------------
{
  shim.loadDictionary(readFileSync(dictPath, 'latin1'));
  const enc = (phones) => (phones ? phones.map(p => p.code + (p.stressed ? '*' : '')).join(' ') : null);
  const words = Object.keys(shim.dictionary);
  const all = words.map(w => enc(P.pronounce(w)));
  const variants = [];
  for (let i = 0; i < n(3000); i++) {
    const w = pick(words);
    const v = weighted([[3, () => w.toUpperCase()], [2, () => w[0].toUpperCase() + w.slice(1)], [1, () => w + 'x'], [1, () => ' ' + w]]);
    variants.push([v, enc(P.pronounce(v)), P.hasWord(v)]);
  }
  const texts = [];
  const punct = [...',.!?;:"()[]-\'', '...', ' - ', '\u{1F600}', '3', '\t', '\n'];
  for (let i = 0; i < n(2000); i++) {
    let t = '';
    for (let j = 0; j < int(0, 12); j++) t += (chance(0.8) ? pick(words) : pick(punct)) + pick([' ', ' ', '  ', '\t', '', '\n']);
    if (chance(0.3)) t = t.toUpperCase();
    texts.push([t, P.pronounceText(t).map(x => [x.word, enc(x.phones)]), enc(P.textToPhonemes(t))]);
  }
  writeJson('pronounce.json', { words, all, variants, texts });
}

console.log(`fuzz fixtures (seed ${SEED}, scale ${SCALE}) written to ${outDir}`);
