// Exact-parity tests against fixtures recorded from the JS engine.
//
// Generate the fixtures first (needs Node and the klattsch-main checkout next
// to this repository):
//
//   node tools/gen-fixtures.mjs

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use klattsch_core::*;
use serde_json::Value as J;

// --- DSP ----------------------------------------------------------------------

#[test]
fn js_math_matches_v8() {
    let f = load("dsp.json");
    let mut bad = Vec::new();
    for t in f["trig"].as_array().unwrap() {
        let x = num(&t[0]);
        if klattsch_core::js::sin(x) != num(&t[1]) || klattsch_core::js::cos(x) != num(&t[2]) {
            bad.push(x);
        }
    }
    assert!(bad.is_empty(), "{} sin/cos mismatches, first x = {:e}", bad.len(), bad[0]);
    for p in f["pow"].as_array().unwrap() {
        let (x, y) = (num(&p[0]), num(&p[1]));
        assert_eq!(klattsch_core::js::pow(x, y), num(&p[2]), "pow({x}, {y})");
    }
}

#[test]
fn dsp_primitives() {
    let f = load("dsp.json");
    for g in f["glottal"].as_array().unwrap() {
        let e = num(&g["effort"]);
        for (i, v) in g["values"].as_array().unwrap().iter().enumerate() {
            let got = glottal_pulse(i as f64 / 400.0, e);
            assert_eq!(got, num(v), "glottal effort {e} i {i}");
        }
    }
    for (i, v) in f["glottalDefault"].as_array().unwrap().iter().enumerate() {
        assert_eq!(glottal_pulse(i as f64 / 100.0 + 0.003, 0.5), num(v), "glottal default {i}");
    }
    for x in f["xorshift"].as_array().unwrap() {
        let mut s = x["seed"].as_i64().unwrap() as i32;
        for v in x["seq"].as_array().unwrap() {
            s = xorshift(s);
            assert_eq!(i64::from(s), v.as_i64().unwrap());
        }
    }
    for pair in f["softClip"].as_array().unwrap() {
        assert_eq!(soft_clip(num(&pair[0])), num(&pair[1]), "softClip {}", pair[0]);
    }
    for b in f["biquads"].as_array().unwrap() {
        let ctx = format!("biquad {} {} {}", b["f"], b["bw"], b["sr"]);
        let mut bq = BandpassBiquad::new();
        bq.set_freq(num(&b["f"]), num(&b["bw"]), num(&b["sr"]));
        let coeffs = [bq.b0, bq.b1, bq.b2, bq.a1, bq.a2];
        for (c, e) in coeffs.iter().zip(b["coeffs"].as_array().unwrap()) {
            assert_eq!(*c, num(e), "{ctx} coeffs");
        }
        let mut s = 0x1234567;
        for (i, e) in b["out"].as_array().unwrap().iter().enumerate() {
            s = xorshift(s);
            assert_eq!(bq.process(f64::from(s) / 2147483648.0), num(e), "{ctx} sample {i}");
        }
        bq.reset();
        assert_eq!(bq.process(0.5), num(&b["afterReset"]), "{ctx} reset");
    }
}

// --- banks ----------------------------------------------------------------------

fn fresh_registry_like_fixtures(f: &J) -> BankRegistry {
    let reg = BankRegistry::new();
    for b in f["registered"].as_array().unwrap() {
        reg.register(to_value(b)).unwrap();
    }
    reg
}

#[test]
fn banks_match() {
    let f = load("banks.json");
    let reg = BankRegistry::new();
    let before = &f["before"];
    assert_json(&serde_json::json!(reg.list()), &before["list"], "list");
    for (name, expected) in before["resolved"].as_object().unwrap() {
        let b = reg.get(name).unwrap().unwrap();
        assert_json(&b.to_json(), expected, name);
    }
    assert_json(&serde_json::json!(phoneme_keys()), &before["phonemeKeys"], "PHONEME_KEYS");
    assert!(reg.get("does-not-exist").unwrap().is_none());

    for pair in f["registerErrors"].as_array().unwrap() {
        let err = reg.register(to_value(&pair[0])).unwrap_err();
        assert_eq!(err.message(), pair[1].as_str().unwrap());
    }

    let reg = fresh_registry_like_fixtures(&f);
    let after = &f["after"];
    assert_json(&serde_json::json!(reg.list()), &after["list"], "list after");
    for (name, expected) in after["resolved"].as_object().unwrap() {
        assert_json(&reg.get(name).unwrap().unwrap().to_json(), expected, name);
    }
    for (name, msg) in after["errors"].as_object().unwrap() {
        assert_eq!(reg.get(name).unwrap_err().message(), msg.as_str().unwrap(), "{name}");
    }

    let errs = &f["resolveErrors"];
    let r = |b: BankRef| reg.resolve(Some(&b)).unwrap_err().message().to_string();
    assert_eq!(r(BankRef::Name("unknown-name".into())), errs[0][1].as_str().unwrap());
    assert_eq!(r(BankRef::Value(klattsch_core::Value::Num(5.0))), errs[1][1].as_str().unwrap());
    let no_name = serde_json::json!({ "extends": "klatt1980-en", "phonemes": {} });
    assert_eq!(r(BankRef::Value(to_value(&no_name))), errs[2][1].as_str().unwrap());
}

// --- tokenizer / compiler ---------------------------------------------------------

#[test]
fn tokenize_matches() {
    for case in load("tokenize.json").as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let t = tokenize(input);
        assert_eq!(t.source, case["source"].as_str().unwrap(), "source for {input:?}");
        let tokens = J::Array(t.tokens.iter().map(Token::to_json).collect());
        assert_json(&tokens, &case["tokens"], &format!("tokens for {input:?}"));
    }
}

fn compile_options(o: &J) -> CompileOptions {
    let f = |k: &str| o.get(k).and_then(J::as_f64);
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
            other => BankRef::Value(to_value(other)),
        }),
        engine: o.get("engine").and_then(J::as_str).map(String::from),
        extras: o.get("extras").and_then(J::as_object).map_or_else(Map::new, |m| {
            m.iter().map(|(k, v)| (k.clone(), to_value(v))).collect()
        }),
    }
}

// Compile every fixture case in order (bank registrations made by earlier
// cases are visible to later ones, as in the JS run).
fn compiled_cases() -> Vec<(String, J, Result<CompileResult>)> {
    let reg = fresh_registry_like_fixtures(&load("banks.json"));
    load("compile.json")
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let input = case["input"].as_str().unwrap();
            let r = compile_string_with_registry(input, &compile_options(&case["opts"]), &reg);
            (case["id"].as_str().unwrap().to_string(), case.clone(), r)
        })
        .collect()
}

#[test]
fn compile_matches() {
    for (id, case, result) in compiled_cases() {
        let ctx = format!("{id} {:?} ({})", case["input"].as_str().unwrap(), case["optName"]);
        match (&result, case["error"].as_str()) {
            (Ok(r), None) => assert_json(&r.to_json(), &case["result"], &ctx),
            (Err(e), Some(msg)) => assert_eq!(e.message(), msg, "{ctx}"),
            (Ok(_), Some(msg)) => panic!("{ctx}: expected error {msg}"),
            (Err(e), None) => panic!("{ctx}: unexpected error {e}"),
        }
    }
}

// The AudioWorklet wrapper (src/formant-worklet.js) run in Node, replayed
// through FormantProcessor block by block.
#[test]
fn worklet_matches() {
    for sc in load("worklet.json").as_array().unwrap() {
        let name = sc["name"].as_str().unwrap();
        let got = run_worklet(sc);
        for (v, file) in sc["files"].as_array().unwrap().iter().enumerate() {
            match (file.as_str(), &got[v]) {
                (Some(f), Some(samples)) => assert_samples(samples, &f32s(f), &format!("worklet {name} output {v}")),
                (None, None) => {}
                _ => panic!("worklet {name} output {v}: channel layout differs"),
            }
        }
    }
}

// The JS `compile(tokensArray)` form: no source text, offsets kept.
#[test]
fn legacy_token_array_compile_matches() {
    for case in load("legacy.json").as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let parsed = Tokenized { tokens: tokenize(input).tokens, source: String::new() };
        let r = compile(&parsed, &CompileOptions::default()).unwrap();
        assert_json(&r.to_json(), &case["result"], &format!("legacy {input:?}"));
    }
}

#[test]
fn render_matches() {
    let compiled = compiled_cases();
    for r in load("render.json").as_array().unwrap() {
        let id = r["id"].as_str().unwrap();
        let sr = num(&r["sr"]);
        let (_, case, result) = compiled.iter().find(|(cid, ..)| cid == id).unwrap();
        let result = result.as_ref().unwrap();
        for (vi, file) in r["voices"].as_array().unwrap().iter().enumerate() {
            let v = &result.voices[vi];
            let Some(file) = file.as_str() else {
                assert!(v.schedule.is_empty());
                continue;
            };
            let got = render_to_buffer(sr, &v.schedule, Some(v.total_ms), None).unwrap();
            let ctx = format!("render {id} {:?} voice {vi} @ {sr}", case["input"].as_str().unwrap());
            assert_samples(&got, &f32s(file), &ctx);
        }
    }
}

#[test]
fn cli_wav_matches() {
    for c in load("cli.json").as_array().unwrap() {
        let text = c["text"].as_str().unwrap();
        let result = compile_string(text, &CompileOptions::default()).unwrap();
        let buf = render_mix(&result, 48000.0).unwrap();
        let wav = encode_wav(&buf, 48000, &WavOptions {
            metadata: Some(WavMetadata {
                software: Some("klattsch · https://tgies.github.io/klattsch".into()),
                comment: Some(text.into()),
            }),
            ..WavOptions::default()
        });
        assert_eq!(wav.gain, num(&c["gain"]), "gain for {text:?}");
        assert!(wav.bytes == bin(c["wav"].as_str().unwrap()), "wav bytes for {text:?}");
    }
}

#[test]
fn wav_encoder_matches() {
    let src = f32s("wav-src.f32");
    for w in load("wav.json").as_array().unwrap() {
        let name = w["name"].as_str().unwrap();
        let opt = &w["opt"];
        let samples = match w.get("zeros") {
            Some(n) => vec![0.0f32; n.as_u64().unwrap() as usize],
            None => src.clone(),
        };
        let metadata = opt.get("metadata").map(|m| WavMetadata {
            software: m.get("software").and_then(J::as_str).map(String::from),
            comment: m.get("comment").and_then(J::as_str).map(String::from),
        });
        let peak_normalize = match opt.get("peakNormalize") {
            Some(p) => Some(num(p)),
            None => Some(0.95),
        };
        let sr = w["sampleRate"].as_u64().unwrap() as u32;
        let wav = encode_wav(&samples, sr, &WavOptions { peak_normalize, metadata });
        assert_eq!(wav.gain, num(&w["gain"]), "gain {name}");
        assert!(wav.bytes == bin(w["wav"].as_str().unwrap()), "bytes {name}");
    }
}

// --- live synth ---------------------------------------------------------------------

fn partial(o: &J) -> PartialParams {
    PartialParams::from_map(&o.as_object().unwrap().iter().map(|(k, v)| (k.clone(), to_value(v))).collect())
}

fn synth_events(events: &J) -> Vec<SynthEvent> {
    events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| SynthEvent {
            at_ms: e.get("atMs").map(num),
            target: partial(&e["target"]),
            transition_ms: e.get("transitionMs").map(num),
        })
        .collect()
}

#[test]
fn synth_scenarios_match() {
    let f = load("synth.json");
    for sc in f["scenarios"].as_array().unwrap() {
        let name = sc["name"].as_str().unwrap();
        let ctor = &sc["ctor"];
        let init = ctor.get("initialTarget").map(partial);
        let mut synth = FormantSynth::new(num(&sc["sr"]), init.as_ref()).unwrap();
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
                    let ms = op.get("transitionMs").map_or(synth::DEFAULT_TRANSITION_MS, num);
                    synth.set_target(&partial(&op["target"]), ms);
                }
                "queueSchedule" => {
                    let start = op.get("startCounter").map_or(0.0, num);
                    synth.queue_events(&synth_events(&op["events"]), start);
                }
                "reset" => synth.reset(op.get("initialTarget").map(partial).as_ref()),
                other => panic!("unknown op {other}"),
            }
        }
        assert_samples(&out, &f32s(sc["output"].as_str().unwrap()), &format!("scenario {name}"));
    }

    for e in f["errors"].as_array().unwrap() {
        let sr = match e[0].as_str().unwrap() {
            "undefined" | "NaN" => f64::NAN,
            s => s.parse().unwrap(),
        };
        assert_eq!(FormantSynth::new(sr, None).unwrap_err().message(), e[1].as_str().unwrap());
    }
    let rtb_err = f["rtbErrors"][0].as_str().unwrap();
    assert_eq!(render_to_buffer(48000.0, &[], None, None).unwrap_err().message(), rtb_err);

    let schedule = compile_string("HH AH L OW", &CompileOptions::default()).unwrap();
    assert_json(
        &J::Array(schedule.schedule().iter().map(ScheduleEvent::to_json).collect()),
        &f["rtbSchedule"],
        "rtb schedule",
    );
    let got = render_to_buffer(48000.0, schedule.schedule(), None, None).unwrap();
    assert_samples(&got, &f32s(f["rtbDefault"].as_str().unwrap()), "renderToBuffer default totalMs");
}

// --- text front ends ----------------------------------------------------------------

#[test]
fn kana_matches() {
    for c in load("kana.json").as_array().unwrap() {
        let input = c["input"].as_str().unwrap();
        assert_json(&phones_json(&kana_to_phonemes(input)), &c["kana"], &format!("kana {input:?}"));
        assert_json(&phones_json(&romaji_to_phonemes(input)), &c["romaji"], &format!("romaji {input:?}"));
        assert_eq!(is_kana(input), c["isKana"].as_bool().unwrap(), "isKana {input:?}");
        assert_json(&phones_json(&japanese_to_phonemes(input)), &c["japanese"], &format!("japanese {input:?}"));
    }
}

#[test]
fn pronounce_matches() {
    let f = load("pronounce.json");
    let dict_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/cmudict/cmudict-0.7b");
    let dict = CmuDict::load(dict_path).unwrap();
    assert_eq!(dict.len() as u64, f["entryCount"].as_u64().unwrap(), "entry count");
    for p in f["pronounce"].as_array().unwrap() {
        let w = p[0].as_str().unwrap();
        assert_json(&phones_json(&dict.pronounce(w)), &p[1], &format!("pronounce {w:?}"));
    }
    for p in f["hasWord"].as_array().unwrap() {
        let w = p[0].as_str().unwrap();
        assert_eq!(dict.has_word(w), p[1].as_bool().unwrap(), "hasWord {w:?}");
    }
    for p in f["pronounceText"].as_array().unwrap() {
        let s = p[0].as_str().unwrap();
        let got: Vec<J> = dict
            .pronounce_text(s)
            .iter()
            .map(|w| serde_json::json!({ "word": w.word, "phones": phones_json(&w.phones) }))
            .collect();
        assert_json(&J::Array(got), &p[1], &format!("pronounceText {s:?}"));
    }
    for p in f["textToPhonemes"].as_array().unwrap() {
        let s = p[0].as_str().unwrap();
        let got = J::Array(dict.text_to_phonemes(s).iter().map(Phone::to_json).collect());
        assert_json(&got, &p[1], &format!("textToPhonemes {s:?}"));
    }

    let synthetic = &f["synthetic"];
    let dict = CmuDict::parse(
        ";;; comment\nFOO  F UW1\nBAR  B AA2 R0 XYZ3 Q\nBAZ  ABC AH0\nBAZ  B AE1 Z\nQUX  # nothing\nQUUX  ABC XYZ1\n\nODD\tAA1   IY0 # trailing\n",
    );
    for (word, entry) in synthetic["dict"].as_object().unwrap() {
        assert_eq!(dict.entry(word), entry.as_str(), "synthetic entry {word}");
    }
    assert_eq!(dict.len(), synthetic["dict"].as_object().unwrap().len());
    for p in synthetic["pronounce"].as_array().unwrap() {
        let w = p[0].as_str().unwrap();
        assert_json(&phones_json(&dict.pronounce(w)), &p[1], &format!("synthetic {w:?}"));
    }
}

#[test]
fn highlight_matches() {
    // The JS run highlighted after registering the test banks.
    let reg = fresh_registry_like_fixtures(&load("banks.json"));
    let lookup = registry_phonemes_for(&reg);
    for c in load("highlight.json").as_array().unwrap() {
        let input = c["input"].as_str().unwrap();
        let opt_name = c["optName"].as_str().unwrap();
        let opts = match opt_name {
            "none" => HighlightOptions::default(),
            "registry" => HighlightOptions { phonemes_for: Some(&lookup), initial_bank: None },
            "registry-en" => HighlightOptions { phonemes_for: Some(&lookup), initial_bank: Some("klatt1980-en") },
            "registry-ja" => HighlightOptions { phonemes_for: Some(&lookup), initial_bank: Some("ja-mokhtari-2000") },
            "registry-missing" => HighlightOptions { phonemes_for: Some(&lookup), initial_bank: Some("nope") },
            other => panic!("unknown highlight options {other}"),
        };
        let spans = build_highlight(input, &opts);
        let got = J::Array(spans.iter().map(|s| serde_json::json!([s.class, s.text])).collect());
        assert_json(&got, &c["nodes"], &format!("highlight {input:?} ({opt_name})"));
        // Spans tile the input exactly.
        let mut pos = 0;
        for s in &spans {
            assert_eq!(s.start, pos, "span start {input:?}");
            assert_eq!(s.text.chars().count(), s.end - s.start);
            pos = s.end;
        }
        assert_eq!(pos, input.chars().count(), "span coverage {input:?}");
    }
}

#[test]
fn resolved_bank_ref_is_used_directly() {
    let reg = BankRegistry::new();
    let bank: Arc<ResolvedBank> = reg.get("ja-hecko-2026").unwrap().unwrap();
    let opts = CompileOptions { bank: Some(BankRef::Resolved(bank)), ..Default::default() };
    let a = compile_string_with_registry("A I U", &opts, &reg).unwrap();
    let b = compile_string_with_registry(
        "A I U",
        &CompileOptions { bank: Some("ja-hecko-2026".into()), ..Default::default() },
        &reg,
    )
    .unwrap();
    assert_eq!(a, b);
}
