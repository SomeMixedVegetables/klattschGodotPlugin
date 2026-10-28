// Golden-fixture generator: runs the original klattsch JS engine and records
// its outputs so the Rust port can be checked for exact parity.
//
//   node tools/gen-fixtures.mjs [path/to/klattsch-main]
//
// Writes fixtures/*.json and fixtures/bin/*.f32 / *.wav. Source offsets in
// tokens, phrases, and highlight spans are converted from JS UTF-16 code units
// to Unicode code points, which is what the Rust port (and Godot) index by.

import { mkdirSync, mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { registerHooks } from 'node:module';
import { loadWorklet } from './worklet-harness.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const jsRoot = resolve(process.argv[2] ?? join(root, '..', 'klattsch-main'));
const outDir = join(root, 'fixtures');
const binDir = join(outDir, 'bin');
const dictPath = join(root, 'data', 'cmudict', 'cmudict-0.7b');

rmSync(outDir, { recursive: true, force: true });
mkdirSync(binDir, { recursive: true });

// --- CMU dictionary shim ----------------------------------------------------
// pronounce.js imports the npm `cmu-pronouncing-dictionary` package. Redirect
// that import to a shim whose dictionary is filled from cmudict-0.7b using the
// same parsing rules as the Rust loader (see klattsch-core/src/pronounce.rs).
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

// --- helpers ----------------------------------------------------------------
const writeJson = (name, data) => writeFileSync(join(outDir, name), JSON.stringify(data));
const writeF32 = (name, f32) => {
  writeFileSync(join(binDir, name), Buffer.from(f32.buffer, f32.byteOffset, f32.byteLength));
  return name;
};

// Map a UTF-16 offset in `s` to a code-point offset.
function cpOffset(s, u16) {
  let cp = 0;
  for (let i = 0; i < u16 && i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length) {
      const d = s.charCodeAt(i + 1);
      if (d >= 0xDC00 && d <= 0xDFFF) {
        if (i + 1 < u16) i++;
      }
    }
    cp++;
  }
  return cp;
}
const convTokens = (tokens, src) => tokens.map(t => ({
  ...t, srcStart: cpOffset(src, t.srcStart), srcEnd: cpOffset(src, t.srcEnd),
}));
const convPhrases = (phrases, src) => phrases.map(p => ({
  ...p,
  srcStart: cpOffset(src, p.srcStart),
  srcEnd: cpOffset(src, p.srcEnd),
  tokenSrcStart: cpOffset(src, p.tokenSrcStart),
}));
const convCompiled = (r) => ({
  ...r,
  phrases: convPhrases(r.phrases, r.source),
  voices: r.voices.map(v => ({ ...v, phrases: convPhrases(v.phrases, r.source) })),
});
const errMsg = (fn) => { try { fn(); return null; } catch (e) { return String(e.message); } };

// --- custom bank used by several cases ---------------------------------------
const customBank = {
  schemaVersion: 1,
  name: 'test-custom',
  displayName: 'Test custom bank',
  extends: 'klatt1980-en',
  phonemes: {
    AA: null,
    QX: {
      voicing: 1, F1: 500, F2: 1500, F3: 2500, F4: 3500, BW4: 250, F9: 9000,
      BW1: 60, BW2: 90, BW3: 120, A1: 1, A2: 0.8, A3: 0.5, gain: 2.5, note: 'custom',
    },
    QG: {
      voicing: 1, F1: 300, F2: 900, F3: 2200, BW1: 60, BW2: 90, BW3: 120,
      A1: 1, A2: 0.7, A3: 0.4, glideTo: { F1: 600, F2: 1700, F4: 3600 }, F4: 3300,
    },
  },
};
const childBank = {
  schemaVersion: 1, name: 'test-child', extends: 'test-custom',
  phonemes: { QZ: { voicing: 0, F1: 400, F2: 1200, F3: 4000, BW1: 200, BW2: 200, BW3: 300, A1: 0, A2: 0.2, A3: 0.6, isStop: 1 } },
};
const rawResolved = {
  name: 'raw-inline', phonemes: {
    AA: { voicing: 1, F1: 700, F2: 1220, F3: 2600, BW1: 130, BW2: 70, BW3: 160, A1: 1, A2: 0.9, A3: 0.7 },
    IY: null,
  },
};

// --- banks ------------------------------------------------------------------
const bankFixture = { before: {}, after: {} };
bankFixture.before.list = E.banks.list();
bankFixture.before.resolved = Object.fromEntries(E.banks.list().map(n => [n, E.banks.get(n)]));
bankFixture.before.phonemeKeys = E.PHONEME_KEYS;
bankFixture.before.getMissing = E.banks.get('does-not-exist') === undefined;
bankFixture.registerErrors = [
  [null, errMsg(() => E.registerBank(null))],
  [[], errMsg(() => E.registerBank([]))],
  ['str', errMsg(() => E.registerBank('str'))],
  [{ schemaVersion: 1 }, errMsg(() => E.registerBank({ schemaVersion: 1 }))],
  [{ name: 'x', schemaVersion: 2, phonemes: {} }, errMsg(() => E.registerBank({ name: 'x', schemaVersion: 2, phonemes: {} }))],
  [{ name: 'x' }, errMsg(() => E.registerBank({ name: 'x' }))],
];
E.registerBank(customBank);
E.registerBank(childBank);
E.registerBank({ schemaVersion: 1, name: 'cyc-a', extends: 'cyc-b', phonemes: {} });
E.registerBank({ schemaVersion: 1, name: 'cyc-b', extends: 'cyc-a', phonemes: {} });
E.registerBank({ schemaVersion: 1, name: 'orphan', extends: 'nope', phonemes: {} });
bankFixture.registered = [customBank, childBank,
  { schemaVersion: 1, name: 'cyc-a', extends: 'cyc-b', phonemes: {} },
  { schemaVersion: 1, name: 'cyc-b', extends: 'cyc-a', phonemes: {} },
  { schemaVersion: 1, name: 'orphan', extends: 'nope', phonemes: {} }];
bankFixture.after.list = E.banks.list();
bankFixture.after.resolved = Object.fromEntries(
  ['test-custom', 'test-child'].map(n => [n, E.banks.get(n)]));
bankFixture.after.errors = Object.fromEntries(
  ['cyc-a', 'cyc-b', 'orphan'].map(n => [n, errMsg(() => E.banks.get(n))]));
bankFixture.resolveErrors = [
  ['unknown-name', errMsg(() => E.resolveBank('unknown-name'))],
  ['number', errMsg(() => E.resolveBank(5))],
  ['extends-no-name', errMsg(() => E.resolveBank({ extends: 'klatt1980-en', phonemes: {} }))],
];
writeJson('banks.json', bankFixture);

// --- DSP primitives ---------------------------------------------------------
{
  const glottal = [];
  const efforts = [-0.5, 0, 0.1, 0.25, 0.5, 0.73, 1, 1.5];
  for (const e of efforts) {
    const vals = [];
    for (let i = 0; i <= 400; i++) vals.push(D.glottalPulse(i / 400, e));
    glottal.push({ effort: e, values: vals });
  }
  const glottalDefault = [];
  for (let i = 0; i < 100; i++) glottalDefault.push(D.glottalPulse(i / 100 + 0.003));

  const xs = [];
  for (const seed of [0xACE1ACE1 | 0, 1, -1, 123456789, 0x7FFFFFFF, -2147483648]) {
    let s = seed; const seq = [];
    for (let i = 0; i < 2000; i++) { s = D.xorshift(s); seq.push(s); }
    xs.push({ seed, seq });
  }

  const soft = [];
  for (let i = -400; i <= 400; i++) { const x = i / 50; soft.push([x, D.softClip(x)]); }
  for (const x of [0.85, -0.85, 0.8500000001, 1e6, -1e6]) soft.push([x, D.softClip(x)]);

  const biquads = [];
  const configs = [
    [500, 80, 48000], [1500, 120, 44100], [10, 5, 48000], [30000, 400, 48000],
    [2500, 160, 22050], [3000.5, 210.25, 32000], [100, 1000, 48000],
  ];
  for (const [f, bw, sr] of configs) {
    const bq = new D.BandpassBiquad();
    bq.setFreq(f, bw, sr);
    const coeffs = [bq.b0, bq.b1, bq.b2, bq.a1, bq.a2];
    let s = 0x1234567;
    const out = new Float64Array(512);
    for (let i = 0; i < 512; i++) { s = D.xorshift(s); out[i] = bq.process(s / 2147483648); }
    bq.reset();
    const afterReset = bq.process(0.5);
    biquads.push({ f, bw, sr, coeffs, out: [...out], afterReset });
  }
  // Math.sin/cos across small, medium, and huge arguments (the last exercise
  // the Payne-Hanek reduction path), and Math.pow as used for note names.
  const trig = [];
  let ts = 2468;
  const tr = () => { ts = D.xorshift(ts); return (ts >>> 0) / 4294967296; };
  const special = [0, 1e-9, -1e-9, Math.PI / 4, Math.PI / 2, Math.PI, 3 * Math.PI / 4, 2 * Math.PI,
    1e6, -1e6, 1e10, 1e22, 1e300, -1e300, 2 ** 19 * Math.PI / 2, 4.71238898038469, 355, 103993, 5e-324,
    // Hardest known argument reductions: the double closest to a multiple of
    // pi/2 (6381956970095103 * 2^797) and its neighbors.
    ...[-2, -1, 0, 1, 2].flatMap(d => [(6381956970095103 + d) * 2 ** 797, -(6381956970095103 + d) * 2 ** 797]),
    ...[0, 1, 2, 3].map(k => 5.319372648326541e255 * 2 ** k)];
  for (const x of special) trig.push([x, Math.sin(x), Math.cos(x)]);
  for (let i = 0; i < 20000; i++) {
    const u = tr();
    const x = i < 8000 ? u * 7
      : i < 12000 ? (u - 0.5) * 2000
      : i < 16000 ? (u - 0.5) * 2e7
      : (u - 0.5) * 10 ** Math.floor(tr() * 300);
    trig.push([x, Math.sin(x), Math.cos(x)]);
  }
  const pow = [];
  for (let m = -1000; m <= 1000; m++) { const y = (m - 69) / 12; pow.push([2, y, Math.pow(2, y)]); }
  for (const [x, y] of [[3, 2], [2, 0.5], [4, 0.5], [1.5, 3], [10, -2], [0, 5]]) {
    pow.push([x, y, Math.pow(x, y)]);
  }
  writeJson('dsp.json', { glottal, glottalDefault, xorshift: xs, softClip: soft, biquads, trig, pow });
}

// --- tokenizer / compiler cases ---------------------------------------------
const STRINGS = [
  '', '   \n\t ', 'HH AH L OW', 'b140 HH AH L OW', 'bA3 HH AH L OW', 'b=C#4 AA', 'bDb3 AA IY',
  'bA-1 AA', 'bH4 AA', 'bC10 AA', 'b= AA', 'AY+15 D IH D', 'D IH D DH AE(+40) T',
  'r200 bC#4 ( HH AH ) ( L OW )', "HH AH' L OW!", "HH AH ' L OW !", "' AA", '! !',
  'AA , IY ; UW . OW', 'p100 AA p+50 IY p-30 p UW', 'AA p0 IY',
  's1.2 AA s+0.1 IY s UW s-0.3 EH', 'v5 w6 AA m0.5 n3 IY h0.3 t0.5 g0.8 UW',
  'v+2 w-1 m+0.1 n+1 h+0.1 t-0.2 g-0.1 AA v w m n h t g IY', 'r-20 AA r+50 IY r UW r=90 EH',
  'b-10 AA b+20 IY b UW b=-5 EH', 'r80 AA r=+80 IY', 'r0 AA IY', 'r-100 AA',
  '[base=150] AA [pitch=200] IY [rate=90] UW [scale=1.1] EH',
  '[OQ=0.65] AA [FNZ=450] IY [OQ] UW [FNZ] EH', '[F1=800] AA [A1=0] IY [F1] UW',
  '[foo=1] AA [voice] IY [x=1.5.2] UW', '[voice=1] AA', 'AA [voice=1]', 'AA [voice=1] [voice=2] IY',
  'bC3 AA r400 AA AA [voice=1] bC4 IY r400 IY IY', 'b200 AA [voice=1] IY [voice=0] UW',
  '[bank=ja-hecko-2026] K O N N I CH I W A', '[bank=ja-mokhtari-2000] A I U E O [bank] AA',
  '[bank=nope] AA', '[bank] AA', '[engine=foo] AA [engine] IY [engine=bar.v2] UW',
  '# comment\nAA', 'AA #x IY', 'AA#x IY', 'A/* c */A IY', '/* unterminated AA', 'AA /*x*/ IY /**/',
  '/*a*//*b*/AA', 'AA/*', '#', '#\n#\nAA # tail',
  '( AA ( IY ) )', ') AA', '( AA IY', '( )', '( AA ) )', "( HH AH' ) ( L OW+5 )", '( P AY(+20) )',
  'QQ AA', 'hello AA', '123', 'aa AA', 'AA++5 IY+-3 UW(+5', 'AA+5.5 IY-2.25 UW(-1.5)',
  'АА ІҮ', 'Α Ε Η', 'A​A I‍Y ﻿UW', 'ＡＡ ＩＹ', 'b１４０ AA',
  'AA \u{1F600} IY', '\u{1F600}AA IY', 'AA\u0085IY UW', 'AA IY', 'AA᠎IY', 'AA IY　UW',
  'AY OW EY AW OY IY IH EH AE UH UW ER', 'P T K B D G', 'S SH F TH V DH Z ZH HH CH JH',
  'M N NG L R W Y DX _', "B AA' T", 'P AA+30', 'AY(+40) OW-20',
  'b100 r150 v4 w5.5 m0.2 n4 h0.15 t0.3 g0.7 s1.05 HH EH L OW , W ER L D .',
  'QX QG', 'QX IY', '[bank=test-custom] QX QG AA', '[bank=test-child] QX QZ QG IY',
  '[bank=raw-inline] AA',
];
const OPTS = [
  ['default', {}],
  ['opts-voice', { baseF0: 200, rate: 80, scale: 1.1, vibratoDepth: 3, vibratoRate: 6,
    tremoloDepth: 0.2, tremoloRate: 4, aspiration: 0.1, tilt: 0.2, effort: 0.7 }],
  ['opts-engine', { engine: 'myeng', extras: { OQ: 0.5, FNZ: 300 } }],
  ['opts-bank-name', { bank: 'test-custom' }],
  ['opts-bank-raw', { bank: rawResolved }],
  ['opts-bank-new', { bank: { schemaVersion: 1, name: 'test-inline-new', extends: 'klatt1980-en',
    phonemes: { QY: { voicing: 1, F1: 450, F2: 1450, F3: 2450, BW1: 70, BW2: 100, BW3: 150, A1: 1, A2: 0.8, A3: 0.6 } } } }],
];

const tokenize = [];
for (const input of STRINGS) {
  const { tokens, source } = E.tokenize(input);
  tokenize.push({ input, source, tokens: convTokens(tokens, source) });
}
writeJson('tokenize.json', tokenize);

const compile = [];
const renders = [];
const SR_LIST = [48000, 44100, 22050];
let caseIdx = 0;
for (const [optName, opts] of OPTS) {
  const inputs = optName === 'default' ? STRINGS : STRINGS.filter((_, i) => i % 7 === 0 || /Q[XYG]|bank/.test(STRINGS[i]));
  for (const input of inputs) {
    const id = `c${caseIdx++}`;
    let result = null, error = null;
    try { result = convCompiled(E.compileString(input, opts)); } catch (e) { error = String(e.message); }
    compile.push({ id, input, optName, opts, result, error });
    if (!result) continue;
    // Render each voice like bin/klattsch.mjs, at a few sample rates.
    const sampleRates = optName === 'default' ? SR_LIST : [48000];
    for (const sr of sampleRates) {
      const voices = result.voices.map((v, vi) => {
        if (!v.schedule.length) return null;
        const buf = E.renderToBuffer({ sampleRate: sr, schedule: v.schedule, totalMs: v.totalMs });
        return writeF32(`${id}-v${vi}-${sr}.f32`, buf);
      });
      renders.push({ id, sr, voices });
    }
  }
}
writeJson('compile.json', compile);
writeJson('render.json', renders);

// --- CLI-equivalent WAV output ------------------------------------------------
{
  const cli = [];
  const cliInputs = ['HH AH L OW', 'b140 AY+30 . AY-30', 'bC3 AA r400 AA AA [voice=1] bC4 IY r400 IY IY',
    'D IH D DH AE(+40) T', '[voice=1] AA', 'r200 bC#4 ( HH AH ) ( L OW )', 'ДА \u{1F600} ok'];
  for (const [i, text] of cliInputs.entries()) {
    const sampleRate = 48000;
    const { voices, totalMs } = E.compileString(text);
    const buf = new Float32Array(Math.ceil(totalMs * sampleRate / 1000));
    for (const v of voices) {
      if (!v.schedule.length) continue;
      const vb = E.renderToBuffer({ sampleRate, schedule: v.schedule, totalMs: v.totalMs });
      const n = Math.min(buf.length, vb.length);
      for (let j = 0; j < n; j++) buf[j] += vb[j];
    }
    const { bytes, gain } = E.encodeWav(buf, sampleRate, {
      metadata: { software: 'klattsch · https://tgies.github.io/klattsch', comment: text },
    });
    const name = `cli${i}.wav`;
    writeFileSync(join(binDir, name), bytes);
    cli.push({ text, wav: name, gain });
  }
  writeJson('cli.json', cli);
}

// --- the real JS CLI (bin/klattsch.mjs) ----------------------------------------
// Run as a subprocess in a scratch directory with a relative output path, so
// its console output is path-independent. Covers warnings, polyphony, the
// default output name, and the usage error.
{
  const cliPath = join(jsRoot, 'bin', 'klattsch.mjs');
  const runs = [];
  const argSets = [['HH AH L OW', 'out.wav'], ['QQ hello [voice=1] AA , IY', 'out.wav'],
    ['bC3 AA r400 AA AA [voice=1] bC4 IY r400 IY IY', 'out.wav'], ['v5 w6 h0.2 t0.4 g0.9 s1.1 HH EH L OW .'],
    ['ДА \u{1F600} ok', 'out.wav'], []];
  for (const [i, args] of argSets.entries()) {
    const dir = mkdtempSync(join(tmpdir(), 'klattsch-cli-'));
    const r = spawnSync(process.execPath, [cliPath, ...args], { cwd: dir, encoding: 'utf8' });
    const outName = args[1] ?? 'klattsch.wav';
    let wav = null;
    if (r.status === 0) {
      wav = `cli-real${i}.wav`;
      writeFileSync(join(binDir, wav), readFileSync(join(dir, outName)));
    }
    rmSync(dir, { recursive: true, force: true });
    runs.push({ args, status: r.status, stderr: r.stderr, wav });
  }
  writeJson('cli-real.json', runs);
}

// --- WAV encoder options ------------------------------------------------------
{
  const src = new Float32Array(257);
  let s = 99;
  for (let i = 0; i < src.length; i++) { s = D.xorshift(s); src[i] = (s / 2147483648) * 1.7; }
  src[3] = 0.5 / 32767; src[4] = -0.5 / 32767; src[5] = 1.5 / 32767; src[6] = -1.5 / 32767;
  const variants = [
    ['default', {}],
    ['no-normalize', { peakNormalize: 0 }],
    ['half', { peakNormalize: 0.5 }],
    ['meta-both', { metadata: { software: 'abc', comment: 'odd' } }],
    ['meta-even', { metadata: { software: 'ab', comment: 'éé' } }],
    ['meta-soft', { metadata: { software: 'klattsch ·' } }],
    ['meta-empty', { metadata: {} }],
    ['meta-comment-only', { peakNormalize: 0, metadata: { comment: 'AA [voice=1] \u{1F600}' } }],
  ];
  const zeros = new Float32Array(10);
  const wav = [];
  writeF32('wav-src.f32', src);
  for (const [name, opt] of variants) {
    const { bytes, gain } = E.encodeWav(src, 22050, opt);
    writeFileSync(join(binDir, `wav-${name}.wav`), bytes);
    wav.push({ name, opt, sampleRate: 22050, wav: `wav-${name}.wav`, gain });
  }
  const z = E.encodeWav(zeros, 8000);
  writeFileSync(join(binDir, 'wav-zeros.wav'), z.bytes);
  wav.push({ name: 'zeros', opt: {}, sampleRate: 8000, zeros: 10, wav: 'wav-zeros.wav', gain: z.gain });
  writeJson('wav.json', wav);
}

// --- live synth scenarios -----------------------------------------------------
{
  const { schedule } = E.compileString('HH AH L OW');
  const scenarios = [
    { name: 'blocks-128', sr: 48000, ctor: { schedule }, ops: Array(300).fill({ op: 'process', n: 128 }) },
    { name: 'odd-blocks', sr: 44100, ctor: { schedule }, ops: [1, 7, 333, 4096, 1, 2, 9999].map(n => ({ op: 'process', n })) },
    { name: 'set-target', sr: 48000, ctor: { initialTarget: { F0: 140, voicing: 1, A1: 1, A2: 0.5 } }, ops: [
      { op: 'process', n: 1000 },
      { op: 'setTarget', target: { F1: 700, F2: 1200, A3: 0.6, vibratoDepth: 4 }, transitionMs: 50 },
      { op: 'process', n: 3000 },
      { op: 'setTarget', target: { F0: 220, tilt: 0.5, effort: 0.9, tremoloDepth: 0.4 } },
      { op: 'process', n: 500 },
      { op: 'setTarget', target: { voicing: 0, aspiration: 0.4 }, transitionMs: 0 },
      { op: 'process', n: 2000 },
      { op: 'setTarget', target: { gain: 1, unknownKey: 5 }, transitionMs: 0.01 },
      { op: 'process', n: 777 },
    ] },
    { name: 'queue-start-counter', sr: 48000, ctor: {}, ops: [
      { op: 'process', n: 256 },
      { op: 'queueSchedule', events: schedule, startCounter: -2400 },
      { op: 'process', n: 12000 },
      { op: 'queueSchedule', events: schedule, startCounter: 9000 },
      { op: 'process', n: 20000 },
    ] },
    { name: 'reset-mid', sr: 32000, ctor: { schedule }, ops: [
      { op: 'process', n: 5000 },
      { op: 'reset', initialTarget: { F0: 90, voicing: 1, A1: 0.8 } },
      { op: 'process', n: 3000 },
      { op: 'queueSchedule', events: schedule },
      { op: 'process', n: 6000 },
      { op: 'reset' },
      { op: 'process', n: 100 },
    ] },
    { name: 'events-defaults', sr: 16000, ctor: { schedule: [
      { target: { voicing: 1, A1: 1, F0: 100 } },
      { atMs: 40, target: { F1: 900 } },
      { atMs: 40, target: { F1: 300 }, transitionMs: 5 },
      { atMs: 100.7, target: { A1: 0 }, transitionMs: 0 },
    ] }, ops: [{ op: 'process', n: 3000 }] },
    { name: 'extreme', sr: 8000, ctor: { initialTarget: { voicing: 2, F0: 3000, F1: 5000, BW1: 1, A1: 50, gain: 10, vibratoDepth: 2000, vibratoRate: 1000, tilt: 0.95 } },
      ops: [{ op: 'process', n: 4000 }] },
  ];
  const out = [];
  for (const sc of scenarios) {
    const synth = new E.FormantSynth({ sampleRate: sc.sr, ...sc.ctor });
    const chunks = [];
    for (const op of sc.ops) {
      if (op.op === 'process') { const b = new Float32Array(op.n); synth.process(b); chunks.push(b); }
      else if (op.op === 'setTarget') { op.transitionMs === undefined ? synth.setTarget(op.target) : synth.setTarget(op.target, op.transitionMs); }
      else if (op.op === 'queueSchedule') { synth.queueSchedule(op.events, op.startCounter); }
      else if (op.op === 'reset') { synth.reset(op.initialTarget); }
    }
    const total = chunks.reduce((a, b) => a + b.length, 0);
    const all = new Float32Array(total);
    let o = 0; for (const c of chunks) { all.set(c, o); o += c.length; }
    out.push({ ...sc, output: writeF32(`synth-${sc.name}.f32`, all) });
  }
  const errors = [0, -1, undefined, NaN].map(sr => [String(sr), errMsg(() => new E.FormantSynth({ sampleRate: sr }))]);
  const rtbErrors = [errMsg(() => E.renderToBuffer({ schedule: [] })), errMsg(() => E.renderToBuffer({}))];
  const rtbDefault = writeF32('rtb-default.f32', E.renderToBuffer({ schedule }));
  writeJson('synth.json', { scenarios: out, errors, rtbErrors, rtbDefault, rtbSchedule: schedule });
}

// --- kana / romaji ------------------------------------------------------------
{
  const inputs = ['こんにちは', 'コンニチハ', 'きょうと', 'がっこう', 'ラーメン', 'ー', 'っ', 'ちゃっと', 'ゔ', 'abc',
    'konnichiha', 'kyouto', 'gakkou', 'ramen', 'shinbun', 'kon\'nichiwa', 'onna', 'tsukue', 'TOKYO',
    'sushi desu', 'n', 'kn', 'xyz', '--', 'chotto', 'ippai', '', '  ', 'ヴァ', 'カタカナとひらがな', 'Mixed かな',
    '\u{1F600}\u{1F600}', 'ｶﾀｶﾅ', 'ぃ', 'nnn', 'nya', 'kanji 漢字'];
  const kana = inputs.map(t => ({
    input: t,
    kana: K.kanaToPhonemes(t), romaji: K.romajiToPhonemes(t),
    isKana: K.isKana(t), japanese: K.japaneseToPhonemes(t),
  }));
  writeJson('kana.json', kana);
}

// --- pronounce ------------------------------------------------------------------
{
  const text = readFileSync(dictPath, 'latin1');
  shim.loadDictionary(text);
  const words = ['hello', 'HELLO', 'Computer', 'déjà', 'hello(1)', 'a', "don't", "'bout", 'xyzzyq', '', 'read',
    'the', 'quick', 'brown', 'fox', 'naïve', 'co-op', 'NG', 'zzz'];
  const sentences = ['Hello, world!', 'The quick brown fox jumps over the lazy dog.', "I can't believe it's 3 o'clock",
    '  multiple   spaces\tand\nnewlines ', 'xyzzyq hello', '!!! ??? ...', 'déjà vu', ''];
  const pron = {
    pronounce: words.map(w => [w, P.pronounce(w)]),
    hasWord: words.map(w => [w, P.hasWord(w)]),
    pronounceText: sentences.map(s => [s, P.pronounceText(s)]),
    textToPhonemes: sentences.map(s => [s, P.textToPhonemes(s)]),
    entryCount: Object.keys(shim.dictionary).length,
  };
  // A tiny synthetic dictionary exercising odd phone shapes.
  shim.loadDictionary(';;; comment\nFOO  F UW1\nBAR  B AA2 R0 XYZ3 Q\nBAZ  ABC AH0\nBAZ  B AE1 Z\nQUX  # nothing\nQUUX  ABC XYZ1\n\nODD\tAA1   IY0 # trailing\n');
  pron.synthetic = {
    dict: shim.dictionary,
    pronounce: ['foo', 'bar', 'baz', 'qux', 'odd', 'quux', 'none'].map(w => [w, P.pronounce(w)]),
  };
  writeJson('pronounce.json', pron);
}

// --- highlight lexer ------------------------------------------------------------
{
  // Minimal DOM stand-in recording [className | null, text] pieces.
  globalThis.document = {
    createDocumentFragment: () => ({ nodes: [], appendChild(n) { this.nodes.push(n); } }),
    createElement: () => ({ className: '', textContent: '' }),
    createTextNode: (t) => ({ className: null, textContent: t }),
  };
  const H = await import(engineUrl('highlight.js'));
  const reg = E.banks;
  const phonemesFor = (name) => {
    const b = reg.get(name);
    return b ? new Set(Object.keys(b.phonemes)) : null;
  };
  const extra = ['A/*x*/A/*y*/IY', 'b/*c*/140', '[ba/*q*/se=150]', '/* only */ /*two*/', 'AA/* open',
    '[bank=ja-hecko-2026] A K AA', '[bank=nope] AA', '[bank] QQ', 'bH4 bC4 b=Db-1', 'p b r9 x5 ZZ+',
    "AA'+5 IY!(+3) UW'(-2)", '[voice=2] [engine=x] [OQ=1]', '\u{1F600} AA/*\u{1F600}*/IY', 'AA\r\nIY\n',
    '#c\nAA #d', 'AA#e'];
  const optsList = [
    ['none', {}],
    ['registry', { phonemesFor }],
    ['registry-en', { phonemesFor, initialBank: 'klatt1980-en' }],
    ['registry-ja', { phonemesFor, initialBank: 'ja-mokhtari-2000' }],
    ['registry-missing', { phonemesFor, initialBank: 'nope' }],
  ];
  const hl = [];
  for (const input of [...STRINGS, ...extra]) {
    for (const [optName, opts] of optsList) {
      if (optName !== 'none' && optName !== 'registry-en' && !/bank|Q|ZZ|AA'/.test(input)) continue;
      const frag = H.buildHighlight(input, opts);
      const nodes = frag.nodes.map(n => [n.className, n.textContent]);
      const sentinel = nodes.pop();
      if (sentinel[0] !== null || sentinel[1] !== '\n') throw new Error('missing highlight sentinel');
      hl.push({ input, optName, nodes });
    }
  }
  writeJson('highlight.json', hl);
}

// --- AudioWorklet (src/formant-worklet.js) ------------------------------------
{
  const runWorklet = await loadWorklet(jsRoot);
  const hello = E.compileString('HH AH L OW').schedule;
  const chord = E.compileString('bC3 AA r400 AA AA [voice=1] bC4 IY r400 IY IY [voice=2] bG4 UW r300 UW').voices.map(v => v.schedule);
  const sung = E.compileString('r200 bC#4 ( HH AH ) ( L OW )').schedule;
  const scenarios = [
    { name: 'legacy-schedule', sr: 48000, outputs: [1], processorOptions: { schedule: hello }, ops: [{ op: 'process', quanta: 240 }] },
    { name: 'legacy-initial-target', sr: 44100, outputs: [2], processorOptions: { initialTarget: { F0: 150, voicing: 1, A1: 1, A2: 0.6 } },
      ops: [{ op: 'process', quanta: 20 },
        { op: 'message', data: { type: 'frame', target: { F1: 750, F2: 1100, A3: 0.5 }, transitionMs: 60 } },
        { op: 'process', quanta: 40 },
        { op: 'message', data: { type: 'frame', target: { voicing: 0 } } },
        { op: 'process', quanta: 20 }] },
    { name: 'schedules-multi', sr: 48000, outputs: [1, 1, 2, 1], processorOptions: { schedules: [chord[0], null, chord[2], []] },
      ops: [{ op: 'process', quanta: 500 }] },
    { name: 'legacy-plus-schedules', sr: 32000, outputs: [1, 1], processorOptions: { schedule: hello, initialTarget: { F0: 90 }, schedules: [sung, chord[1]] },
      ops: [{ op: 'process', quanta: 300 }] },
    { name: 'messages', sr: 48000, outputs: [1, 1], processorOptions: {},
      ops: [
        { op: 'process', quanta: 5 },
        { op: 'message', data: { type: 'schedule', schedule: chord[1], voice: 1 } },
        { op: 'process', quanta: 30 },
        { op: 'message', data: { type: 'schedule', schedule: hello, startTime: 0.25 } }, // future: delayed
        { op: 'process', quanta: 150 },
        { op: 'message', data: { type: 'schedule', schedule: sung, voice: 0, startTime: 0 } }, // past: early events apply at once
        { op: 'process', quanta: 40 },
        { op: 'message', data: { type: 'frame', target: { F0: 300, vibratoDepth: 8 }, voice: 1, transitionMs: 80 } },
        { op: 'message', data: { type: 'frame', target: { F0: 100 }, voice: null, transitionMs: null } },
        { op: 'process', quanta: 40 },
        { op: 'message', data: { type: 'reset', voice: 1, initialTarget: { F0: 200, voicing: 1, A1: 0.7 } } },
        { op: 'process', quanta: 10 },
        { op: 'message', data: { type: 'bogus', voice: 0 } },
        { op: 'message', data: {} },
        { op: 'message', data: { type: 'reset' } },
        { op: 'process', quanta: 10 },
        { op: 'message', data: { type: 'schedule', schedule: chord[0], voice: 0, startTime: null } },
        { op: 'message', data: { type: 'reset', voice: null, initialTarget: null } },
        { op: 'message', data: { type: 'schedule', schedule: hello, voice: 1, startTime: 0.9 } },
        { op: 'process', quanta: 200 },
      ] },
    { name: 'hidden-voice', sr: 22050, outputs: [1], processorOptions: { schedules: [hello, hello, hello] },
      ops: [{ op: 'message', data: { type: 'schedule', schedule: sung, voice: 2 } },
        { op: 'process', quanta: 50 },
        { op: 'message', data: { type: 'reset' } },
        { op: 'process', quanta: 50 }] },
    { name: 'zero-channel-output', sr: 48000, outputs: [0, 1, 0], processorOptions: { schedules: [hello, sung, chord[2]] },
      ops: [{ op: 'process', quanta: 100 },
        { op: 'message', data: { type: 'frame', target: { F0: 250 }, voice: 0 } },
        { op: 'process', quanta: 100 }] },
    { name: 'no-options', sr: 16000, outputs: [1],
      ops: [{ op: 'message', data: { type: 'schedule', schedule: hello } }, { op: 'process', quanta: 60 }] },
    { name: 'fractional-start', sr: 11025, outputs: [1, 1], processorOptions: {},
      ops: [{ op: 'process', quanta: 3 },
        { op: 'message', data: { type: 'schedule', schedule: hello, voice: 0, startTime: 0.0123456 } },
        { op: 'message', data: { type: 'schedule', schedule: sung, voice: 1, startTime: 0.1 + 1 / 22050 } },
        { op: 'process', quanta: 80 }] },
  ];
  const worklet = [];
  for (const sc of scenarios) {
    const outs = runWorklet(sc);
    const files = outs.map((o, v) => (o ? writeF32(`worklet-${sc.name}-o${v}.f32`, o) : null));
    worklet.push({ ...sc, files });
  }
  writeJson('worklet.json', worklet);
}

// --- legacy compile(tokensArray) form ---------------------------------------------
// ASCII-only inputs: with an array the JS result has source '' but keeps the
// tokens' offsets, so no offset conversion is needed or possible.
writeJson('legacy.json', ['HH AH L OW', 'AA [voice=1] IY , UW', 'b140 ( AA IY ) p50 [OQ=1] EY']
  .map(input => ({ input, result: E.compile(E.tokenize(input).tokens) })));

console.log(`fixtures written to ${outDir}`);
