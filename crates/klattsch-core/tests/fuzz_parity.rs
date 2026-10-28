// Differential fuzz tests: random inputs for every feature, compared against
// the JS engine's outputs recorded by tools/fuzz.mjs.
//
//   node tools/fuzz.mjs [--seed N] [--scale K]
//
// Every case is checked; each test reports all mismatches it finds, not just
// the first.

mod common;

use std::path::PathBuf;

use common::*;
use klattsch_core::*;
use serde_json::{json, Value as J};

fn fuzz(name: &str) -> J {
    load(&format!("fuzz/{name}"))
}

// Numbers as tools/fuzz.mjs writes them: non-finite values are tagged.
fn fnum(v: &J) -> f64 {
    match v {
        J::Number(n) => n.as_f64().unwrap(),
        J::String(s) if s == "-0" => -0.0,
        J::Object(o) => match o["$f"].as_str().unwrap() {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            other => panic!("bad tagged number {other}"),
        },
        other => panic!("not a number: {other}"),
    }
}

fn is_tag(o: &serde_json::Map<String, J>) -> bool {
    o.len() == 1 && o.contains_key("$f")
}

fn to_val(v: &J) -> Value {
    match v {
        J::Object(o) if is_tag(o) => Value::Num(fnum(v)),
        J::Object(o) => Value::Obj(o.iter().map(|(k, x)| (k.clone(), to_val(x))).collect()),
        J::Array(a) => Value::Arr(a.iter().map(to_val).collect()),
        other => Value::from_json(other),
    }
}

fn tnum(x: f64) -> J {
    if x.is_finite() {
        J::Number(serde_json::Number::from_f64(x).unwrap())
    } else {
        let s = if x.is_nan() { "NaN" } else if x > 0.0 { "Infinity" } else { "-Infinity" };
        json!({ "$f": s })
    }
}

fn tval(v: &Value) -> J {
    match v {
        Value::Num(n) => tnum(*n),
        Value::Arr(a) => J::Array(a.iter().map(tval).collect()),
        Value::Obj(m) => tmap(m),
        other => other.to_json(),
    }
}

fn tmap(m: &Map) -> J {
    J::Object(m.iter().map(|(k, v)| (k.clone(), tval(v))).collect())
}

fn same(a: f64, b: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

// Code-point offset -> UTF-16 offset, for comparing against raw JS offsets.
fn u16_map(s: &str) -> Vec<usize> {
    let mut m = Vec::new();
    let mut o = 0;
    for c in s.chars() {
        m.push(o);
        o += c.len_utf16();
    }
    m.push(o);
    m
}

fn hash_f32(v: &[f32]) -> String {
    let (mut h1, mut h2) = (0x811c9dc5u32, 0x9747b28cu32);
    for x in v {
        let mut w = x.to_bits();
        if w & 0x7f800000 == 0x7f800000 && w & 0x007fffff != 0 {
            w = 0x7fc00000;
        }
        h1 = (h1 ^ w).wrapping_mul(0x01000193);
        let old = h2;
        h2 = (old ^ w.rotate_left(16)).wrapping_mul(0x5bd1e995) ^ (old >> 15);
    }
    format!("{h1:x}:{h2:x}:{}", v.len())
}

fn hash_bytes(v: &[u8]) -> String {
    let (mut h1, mut h2) = (0x811c9dc5u32, 0x9747b28cu32);
    for &b in v {
        let w = u32::from(b);
        h1 = (h1 ^ w).wrapping_mul(0x01000193);
        let old = h2;
        h2 = (old ^ (w << 8)).wrapping_mul(0x5bd1e995) ^ (old >> 15);
    }
    format!("{h1:x}:{h2:x}:{}", v.len())
}

// Collects mismatches and fails once at the end with a summary.
struct Check {
    name: &'static str,
    cases: usize,
    failures: Vec<String>,
}

impl Check {
    fn new(name: &'static str) -> Self {
        Self { name, cases: 0, failures: Vec::new() }
    }

    fn case(&mut self, ok: bool, describe: impl FnOnce() -> String) {
        self.cases += 1;
        if !ok {
            self.failures.push(describe());
        }
    }

    fn json(&mut self, actual: &J, expected: &J, ctx: impl FnOnce() -> String) {
        self.cases += 1;
        if let Some(d) = json_diff(actual, expected, "$") {
            self.failures.push(format!("{}: {d}", ctx()));
        }
    }

    fn finish(self) {
        println!("{}: {} checks, {} mismatches", self.name, self.cases, self.failures.len());
        if !self.failures.is_empty() {
            let shown: Vec<_> = self.failures.iter().take(15).collect();
            panic!(
                "{}: {} of {} checks mismatched; first ones:\n{}",
                self.name,
                self.failures.len(),
                self.cases,
                shown.iter().map(|s| format!("  - {s}")).collect::<Vec<_>>().join("\n")
            );
        }
        assert!(self.cases > 0, "{}: no cases ran", self.name);
    }
}

// Replays every bank registration the fuzzer made, in order, into a fresh
// registry, checking registration errors and resolution along the way.
fn replay_banks(check: Option<&mut Check>) -> BankRegistry {
    let reg = BankRegistry::new();
    let mut sink = Check::new("banks");
    let c = check.unwrap_or(&mut sink);
    for (si, sc) in fuzz("banks.json").as_array().unwrap().iter().enumerate() {
        for op in sc["ops"].as_array().unwrap() {
            let got = reg.register(to_val(&op["bank"])).err().map(|e| e.message().to_string());
            let want = op["error"].as_str().map(String::from);
            c.case(got == want, || format!("scenario {si} register {}: {got:?} vs {want:?}", op["bank"]["name"]));
        }
        for lk in sc["lookups"].as_array().unwrap() {
            let name = lk["name"].as_str().unwrap();
            match reg.get(name) {
                Ok(Some(b)) => c.json(&b.to_json(), &lk["resolved"], || format!("scenario {si} get {name}")),
                Ok(None) => c.case(lk["resolved"].is_null() && lk.get("error").is_none(), || {
                    format!("scenario {si} get {name}: missing in Rust")
                }),
                Err(e) => c.case(lk["error"].as_str() == Some(e.message()), || {
                    format!("scenario {si} get {name}: error {e} vs {}", lk["error"])
                }),
            }
        }
        c.json(&json!(reg.list()), &sc["list"], || format!("scenario {si} list"));
    }
    reg
}

#[test]
fn fuzz_banks() {
    let mut c = Check::new("banks");
    replay_banks(Some(&mut c));
    c.finish();
}

fn compile_options(o: &J) -> CompileOptions {
    let f = |k: &str| o.get(k).map(fnum);
    CompileOptions {
        base_f0: f("baseF0"),
        rate: f("rate"),
        scale: f("scale"),
        vibrato_depth: f("vibratoDepth"),
        vibrato_rate: f("vibratoRate"),
        tremolo_depth: f("tremoloDepth"),
        tremolo_rate: f("tremoloRate"),
        aspiration: f("aspiration"),
        tilt: f("tilt"),
        effort: f("effort"),
        bank: o.get("bank").map(|b| match b {
            J::String(s) => BankRef::Name(s.clone()),
            other => BankRef::Value(to_val(other)),
        }),
        engine: o.get("engine").and_then(J::as_str).map(String::from),
        extras: o.get("extras").and_then(J::as_object).map_or_else(Map::new, |m| {
            m.iter().map(|(k, v)| (k.clone(), to_val(v))).collect()
        }),
    }
}

fn schedule_json(s: &[ScheduleEvent]) -> J {
    J::Array(
        s.iter()
            .map(|e| json!({ "atMs": tnum(e.at_ms), "target": tmap(&e.target), "transitionMs": tnum(e.transition_ms) }))
            .collect(),
    )
}

fn phrases_json(p: &[Phrase], m: &[usize]) -> J {
    J::Array(
        p.iter()
            .map(|p| {
                json!({
                    "srcStart": m[p.src_start], "srcEnd": m[p.src_end], "tokenSrcStart": m[p.token_src_start],
                    "tStartMs": tnum(p.t_start_ms), "tEndMs": tnum(p.t_end_ms),
                    "kind": p.kind, "phoneme": p.phoneme,
                })
            })
            .collect(),
    )
}

// The compile result in JS shape, with UTF-16 offsets and tagged numbers.
fn result_json(r: &CompileResult) -> J {
    let m = u16_map(&r.source);
    json!({
        "schedule": schedule_json(r.schedule()),
        "totalMs": tnum(r.total_ms),
        "warnings": r.warnings,
        "phrases": phrases_json(r.phrases(), &m),
        "engine": r.engine(),
        "source": r.source,
        "voices": r.voices.iter().map(|v| json!({
            "schedule": schedule_json(&v.schedule),
            "totalMs": tnum(v.total_ms),
            "phrases": phrases_json(&v.phrases, &m),
            "engine": v.engine,
        })).collect::<Vec<_>>(),
    })
}

#[test]
fn fuzz_tokenize_compile_render() {
    let reg = replay_banks(None);
    let mut tok = Check::new("tokenize");
    let mut comp = Check::new("compile");
    let mut rend = Check::new("render");
    for (i, case) in fuzz("compile.json").as_array().unwrap().iter().enumerate() {
        let input = case["input"].as_str().unwrap();
        let ctx = || format!("case {i} {input:?}");

        let t = tokenize(input);
        let m = u16_map(&t.source);
        let tokens: Vec<J> = t
            .tokens
            .iter()
            .map(|tk| {
                let mut j = tk.to_json();
                match &tk.kind {
                    TokenKind::Directive(d) => {
                        if let Some(v) = d.value {
                            j["value"] = tnum(v);
                        }
                    }
                    TokenKind::Phoneme(p) => j["pitchDelta"] = tnum(p.pitch_delta),
                    _ => {}
                }
                j["srcStart"] = json!(m[tk.src_start]);
                j["srcEnd"] = json!(m[tk.src_end]);
                j
            })
            .collect();
        tok.json(&json!({ "source": t.source, "tokens": tokens }),
                 &json!({ "source": case["source"], "tokens": case["tokens"] }), ctx);

        let result = compile_string_with_registry(input, &compile_options(&case["opts"]), &reg);
        let r = match (result, case["error"].as_str()) {
            (Ok(r), None) => {
                comp.json(&result_json(&r), &case["result"], ctx);
                r
            }
            (Err(e), Some(want)) => {
                comp.case(e.message() == want, || format!("{}: error {e} vs {want}", ctx()));
                continue;
            }
            (Ok(_), Some(want)) => {
                comp.case(false, || format!("{}: expected error {want}", ctx()));
                continue;
            }
            (Err(e), None) => {
                comp.case(false, || format!("{}: unexpected error {e}", ctx()));
                continue;
            }
        };

        let Some(renders) = case.get("renders").and_then(J::as_array) else { continue };
        let sr = fnum(&case["sr"]);
        for (vi, want) in renders.iter().enumerate() {
            let v = &r.voices[vi];
            if want == "skipped" {
                continue;
            }
            if want.is_null() {
                rend.case(v.schedule.is_empty(), || format!("{} voice {vi}: expected empty schedule", ctx()));
                continue;
            }
            let got = render_to_buffer(sr, &v.schedule, Some(v.total_ms), None);
            match (got, want) {
                (Ok(buf), J::String(h)) => {
                    let gh = hash_f32(&buf);
                    rend.case(&gh == h, || format!("{} voice {vi} @ {sr}: hash {gh} vs {h}", ctx()));
                }
                (Err(e), J::Object(o)) => rend.case(o["error"].as_str() == Some(e.message()), || {
                    format!("{} voice {vi}: error {e} vs {}", ctx(), o["error"])
                }),
                (got, want) => rend.case(false, || format!("{} voice {vi}: {:?} vs {want}", ctx(), got.err())),
            }
        }
    }
    tok.finish();
    comp.finish();
    rend.finish();
}

#[test]
fn fuzz_highlight() {
    let reg = replay_banks(None);
    let lookup = registry_phonemes_for(&reg);
    let mut c = Check::new("highlight");
    for (i, case) in fuzz("highlight.json").as_array().unwrap().iter().enumerate() {
        let input = case["input"].as_str().unwrap();
        let opts = match case["mode"].as_str().unwrap() {
            "none" => HighlightOptions::default(),
            "registry" => HighlightOptions { phonemes_for: Some(&lookup), initial_bank: None },
            _ => HighlightOptions { phonemes_for: Some(&lookup), initial_bank: case["initialBank"].as_str() },
        };
        let spans = build_highlight(input, &opts);
        let got = J::Array(spans.iter().map(|s| json!([s.class, s.text])).collect());
        c.json(&got, &case["nodes"], || format!("case {i} {input:?} ({})", case["mode"]));
    }
    c.finish();
}

fn partial(o: &J) -> PartialParams {
    PartialParams::from_map(&o.as_object().unwrap().iter().map(|(k, v)| (k.clone(), to_val(v))).collect())
}

fn synth_events(events: &J) -> Vec<SynthEvent> {
    events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| SynthEvent {
            at_ms: e.get("atMs").map(fnum),
            target: partial(&e["target"]),
            transition_ms: e.get("transitionMs").map(fnum),
        })
        .collect()
}

#[test]
fn fuzz_live_synth() {
    let mut c = Check::new("live synth");
    for (i, sc) in fuzz("synth.json").as_array().unwrap().iter().enumerate() {
        let ctor = &sc["ctor"];
        let init = ctor.get("initialTarget").map(partial);
        let mut synth = FormantSynth::new(fnum(&sc["sr"]), init.as_ref()).unwrap();
        if let Some(s) = ctor.get("schedule") {
            synth.queue_events(&synth_events(s), 0.0);
        }
        let mut out: Vec<f32> = Vec::new();
        for op in sc["ops"].as_array().unwrap() {
            match op["op"].as_str().unwrap() {
                "process" => {
                    let mut b = vec![0.0f32; op["n"].as_u64().unwrap() as usize];
                    synth.process(&mut b);
                    out.extend(b);
                }
                "setTarget" => {
                    let ms = op.get("transitionMs").map_or(synth::DEFAULT_TRANSITION_MS, fnum);
                    synth.set_target(&partial(&op["target"]), ms);
                }
                "queueSchedule" => {
                    let start = op.get("startCounter").map_or(0.0, fnum);
                    synth.queue_events(&synth_events(&op["events"]), start);
                }
                _ => synth.reset(op.get("initialTarget").map(partial).as_ref()),
            }
        }
        let h = hash_f32(&out);
        let want = sc["hash"].as_str().unwrap();
        c.case(h == want, || format!("scenario {i} @ {}: hash {h} vs {want}", sc["sr"]));
    }
    c.finish();
}

#[test]
fn fuzz_worklet() {
    let mut c = Check::new("worklet");
    for (i, sc) in fuzz("worklet.json").as_array().unwrap().iter().enumerate() {
        let got = run_worklet(sc);
        for (v, want) in sc["hashes"].as_array().unwrap().iter().enumerate() {
            let h = got[v].as_deref().map(hash_f32);
            c.case(h.as_deref() == want.as_str(), || format!("scenario {i} output {v}: {h:?} vs {want}"));
        }
    }
    c.finish();
}

#[test]
fn fuzz_wav() {
    let mut c = Check::new("wav");
    for (i, w) in fuzz("wav.json").as_array().unwrap().iter().enumerate() {
        let samples: Vec<f32> = w["samples"].as_array().unwrap().iter().map(|x| fnum(x) as f32).collect();
        let opt = &w["opt"];
        let peak_normalize = match opt.get("peakNormalize") {
            None => Some(0.95),
            Some(J::Null) => None,
            Some(p) => Some(fnum(p)),
        };
        let metadata = opt.get("metadata").map(|m| WavMetadata {
            software: m.get("software").and_then(J::as_str).map(String::from),
            comment: m.get("comment").and_then(J::as_str).map(String::from),
        });
        let sr = w["sampleRate"].as_u64().unwrap() as u32;
        let wav = encode_wav(&samples, sr, &WavOptions { peak_normalize, metadata });
        let want_gain = fnum(&w["gain"]);
        let h = hash_bytes(&wav.bytes);
        c.case(same(wav.gain, want_gain) && h == w["hash"].as_str().unwrap(), || {
            format!("case {i} ({} samples, opt {opt}): gain {} vs {want_gain}, hash {h} vs {}", samples.len(), wav.gain, w["hash"])
        });
    }
    c.finish();
}

#[test]
fn fuzz_dsp() {
    let f = fuzz("dsp.json");
    let mut c = Check::new("glottal/softclip/trig");
    for g in f["glottal"].as_array().unwrap() {
        let (p, e, want) = (fnum(&g[0]), fnum(&g[1]), fnum(&g[2]));
        let got = glottal_pulse(p, e);
        c.case(same(got, want), || format!("glottal({p}, {e}) = {got} vs {want}"));
    }
    for s in f["soft"].as_array().unwrap() {
        let (x, want) = (fnum(&s[0]), fnum(&s[1]));
        let got = soft_clip(x);
        c.case(same(got, want), || format!("softClip({x}) = {got} vs {want}"));
    }
    for t in f["trig"].as_array().unwrap() {
        let x = fnum(&t[0]);
        let (s, co) = (klattsch_core::js::sin(x), klattsch_core::js::cos(x));
        c.case(same(s, fnum(&t[1])) && same(co, fnum(&t[2])), || format!("sin/cos({x:e})"));
    }
    c.finish();

    let mut c = Check::new("biquad");
    for (i, b) in f["biquads"].as_array().unwrap().iter().enumerate() {
        let mut bq = BandpassBiquad::new();
        let mut s = b["seed"].as_i64().unwrap() as i32;
        for (j, st) in b["steps"].as_array().unwrap().iter().enumerate() {
            bq.set_freq(fnum(&st["f"]), fnum(&st["bw"]), fnum(&st["sr"]));
            let mut ok = true;
            for want in st["outs"].as_array().unwrap() {
                s = xorshift(s);
                ok &= same(bq.process(f64::from(s) / 2147483648.0), fnum(want));
            }
            let coeffs = [bq.b0, bq.b1, bq.b2, bq.a1, bq.a2];
            for (k, want) in st["coeffs"].as_array().unwrap().iter().enumerate() {
                ok &= same(coeffs[k], fnum(want));
            }
            c.case(ok, || format!("biquad {i} step {j} (f {} bw {} sr {})", st["f"], st["bw"], st["sr"]));
            if st["reset"].as_bool().unwrap() {
                bq.reset();
            }
        }
    }
    c.finish();
}

#[test]
fn fuzz_kana() {
    let mut c = Check::new("kana/romaji");
    for case in fuzz("kana.json").as_array().unwrap() {
        let s = case["input"].as_str().unwrap();
        let got = json!({
            "kana": phones_json(&kana_to_phonemes(s)),
            "romaji": phones_json(&romaji_to_phonemes(s)),
            "isKana": is_kana(s),
            "japanese": phones_json(&japanese_to_phonemes(s)),
        });
        let want = json!({ "kana": case["kana"], "romaji": case["romaji"], "isKana": case["isKana"], "japanese": case["japanese"] });
        c.json(&got, &want, || format!("{s:?}"));
    }
    c.finish();
}

fn enc(p: Option<&[Phone]>) -> J {
    match p {
        Some(p) => J::String(
            p.iter().map(|x| format!("{}{}", x.code, if x.stressed { "*" } else { "" })).collect::<Vec<_>>().join(" "),
        ),
        None => J::Null,
    }
}

#[test]
fn fuzz_pronounce() {
    let f = fuzz("pronounce.json");
    let dict =
        CmuDict::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/cmudict/cmudict-0.7b")).unwrap();
    let mut c = Check::new("pronounce");
    let words = f["words"].as_array().unwrap();
    c.case(dict.len() == words.len(), || format!("entry count {} vs {}", dict.len(), words.len()));
    for (w, want) in words.iter().zip(f["all"].as_array().unwrap()) {
        let w = w.as_str().unwrap();
        let got = enc(dict.pronounce(w).as_deref());
        c.case(&got == want, || format!("pronounce({w:?}) = {got} vs {want}"));
    }
    for v in f["variants"].as_array().unwrap() {
        let w = v[0].as_str().unwrap();
        let got = enc(dict.pronounce(w).as_deref());
        c.case(got == v[1] && dict.has_word(w) == v[2].as_bool().unwrap(), || format!("variant {w:?}"));
    }
    for t in f["texts"].as_array().unwrap() {
        let text = t[0].as_str().unwrap();
        let words: Vec<J> =
            dict.pronounce_text(text).iter().map(|w| json!([w.word, enc(w.phones.as_deref())])).collect();
        c.json(&J::Array(words), &t[1], || format!("pronounceText({text:?})"));
        let got = enc(Some(&dict.text_to_phonemes(text)));
        c.case(got == t[2], || format!("textToPhonemes({text:?}) = {got} vs {}", t[2]));
    }
    c.finish();
}
