// Measures how much of the original JS engine the parity inputs exercise.
// Runs gen-fixtures.mjs, fuzz.mjs and the JS CLI under V8 block coverage
// (NODE_V8_COVERAGE, no extra packages) and lists every engine source line
// containing code that never ran. Code that never runs is behavior the
// parity tests have not compared.
//
//   node tools/coverage.mjs [path/to/klattsch-main]

import { spawnSync } from 'node:child_process';
import { mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const jsRoot = resolve(process.argv[2] ?? join(here, '..', '..', 'klattsch-main'));
const covDir = mkdtempSync(join(tmpdir(), 'klattsch-cov-'));
const scratch = mkdtempSync(join(tmpdir(), 'klattsch-cli-'));
const env = { ...process.env, NODE_V8_COVERAGE: covDir };

const run = (args, expectFail = false) => {
  const r = spawnSync(process.execPath, args, { env, encoding: 'utf8' });
  if ((r.status !== 0) !== expectFail) {
    throw new Error(`${args.join(' ')} exited ${r.status}\n${r.stderr}`);
  }
};
run([join(here, 'gen-fixtures.mjs'), jsRoot]);
run([join(here, 'fuzz.mjs'), jsRoot]);
const cli = join(jsRoot, 'bin', 'klattsch.mjs');
run([cli, 'HH AH L OW', join(scratch, 'a.wav')]);
run([cli, 'QQ [voice=1] AA', join(scratch, 'b.wav')]);
run([cli], true);
rmSync(scratch, { recursive: true, force: true });

// Merge block coverage from every process: a character counts as covered if
// any process executed it. Within a process, nested ranges override outer ones.
const engineFiles = new Map(); // url -> Int8Array (1 covered, 0 not, -1 unknown)
for (const f of readdirSync(covDir)) {
  const { result } = JSON.parse(readFileSync(join(covDir, f), 'utf8'));
  for (const script of result) {
    if (!script.url.startsWith(pathToFileURL(jsRoot).href + '/') || !script.url.endsWith('.js') && !script.url.endsWith('.mjs')) continue;
    const src = readFileSync(fileURLToPath(script.url), 'utf8');
    if (!engineFiles.has(script.url)) engineFiles.set(script.url, new Int8Array(src.length).fill(-1));
    const merged = engineFiles.get(script.url);
    const counts = new Float64Array(src.length).fill(-1);
    const ranges = script.functions.flatMap(fn => fn.ranges)
      .sort((a, b) => a.startOffset - b.startOffset || b.endOffset - a.endOffset);
    for (const r of ranges) counts.fill(r.count, r.startOffset, Math.min(r.endOffset, src.length));
    for (let i = 0; i < src.length; i++) {
      if (counts[i] > 0) merged[i] = 1;
      else if (counts[i] === 0 && merged[i] !== 1) merged[i] = 0;
    }
  }
}
rmSync(covDir, { recursive: true, force: true });

let anyGaps = false;
for (const [url, cov] of [...engineFiles].sort()) {
  const src = readFileSync(fileURLToPath(url), 'utf8');
  const rel = fileURLToPath(url).slice(jsRoot.length + 1).replaceAll('\\', '/');
  let total = 0, missed = 0;
  const missedLines = new Map();
  let line = 1;
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (c === '\n') { line++; continue; }
    if (/\s/.test(c) || cov[i] === -1) continue;
    total++;
    if (cov[i] === 0) {
      missed++;
      missedLines.set(line, (missedLines.get(line) ?? '') + c);
    }
  }
  const pct = total ? (100 * (total - missed) / total).toFixed(2) : '100.00';
  console.log(`${rel}: ${pct}% of code executed${missed ? ` (${missed} chars never ran)` : ''}`);
  const lines = src.split('\n');
  for (const [ln, chars] of missedLines) {
    anyGaps = true;
    console.log(`  line ${ln}: ${lines[ln - 1].trim()}    [unrun: ${chars.slice(0, 60)}]`);
  }
}
process.exitCode = anyGaps ? 1 : 0;
