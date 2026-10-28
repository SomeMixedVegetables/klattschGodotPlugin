// Tests for the public Rust API surface that has no JS counterpart to compare
// against: runtime bank loading, typed synth parameters, accessors, and value
// conversions.

use std::sync::Arc;

use klattsch_core::*;

const CUSTOM_BANK: &str = r#"{
  "schemaVersion": 1,
  "name": "api-test-voice",
  "displayName": "API test voice",
  "extends": "klatt1980-en",
  "phonemes": {
    "QX": { "voicing": 1, "F1": 500, "F2": 1500, "F3": 2500, "BW1": 60, "BW2": 90, "BW3": 120,
            "A1": 1, "A2": 0.8, "A3": 0.5 }
  }
}"#;

#[test]
fn custom_bank_loads_from_file_and_json() {
    let path = std::env::temp_dir().join(format!("klattsch-api-test-{}.json", std::process::id()));
    std::fs::write(&path, CUSTOM_BANK).unwrap();
    let from_file = BankRegistry::new();
    assert_eq!(from_file.load_file(&path).unwrap(), "api-test-voice");
    std::fs::remove_file(&path).unwrap();

    let from_json = BankRegistry::new();
    assert_eq!(from_json.register_json(CUSTOM_BANK).unwrap(), "api-test-voice");

    let bank = from_file.get("api-test-voice").unwrap().unwrap();
    assert_eq!(bank.name_str(), Some("api-test-voice"));
    assert_eq!(bank.display_name_str(), Some("API test voice"));
    assert!(bank.phoneme("QX").is_some());
    assert!(bank.phoneme("AA").is_some(), "inherits from klatt1980-en");

    let opts = CompileOptions::default();
    let a = compile_string_with_registry("[bank=api-test-voice] QX AA", &opts, &from_file).unwrap();
    let b = compile_string_with_registry("[bank=api-test-voice] QX AA", &opts, &from_json).unwrap();
    assert_eq!(a, b);
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);

    let missing = from_file.load_file(std::env::temp_dir().join("klattsch-no-such-bank.json")).unwrap_err();
    assert!(missing.message().starts_with("cannot read bank "), "{missing}");
    assert!(from_file.register_json("{ nope").unwrap_err().message().starts_with("invalid JSON"));
    assert_eq!(
        from_file.register_json(r#"{"name": "x", "schemaVersion": 2}"#).unwrap_err().message(),
        "registerBank: unsupported schemaVersion 2 (expected 1)"
    );
}

#[test]
fn bank_refs_and_globals() {
    let reg = BankRegistry::default();
    assert_eq!(reg.list(), BankRegistry::new().list());
    assert_eq!(reg.default_name(), DEFAULT_BANK);
    let default = reg.resolve(None).unwrap();
    assert_eq!(reg.resolve(Some(&BankRef::Value(Value::Null))).unwrap(), default);
    let by_name = reg.resolve(Some(&"ja-hecko-2026".into())).unwrap();
    let by_value = reg.resolve(Some(&BankRef::Value(Value::from("ja-hecko-2026")))).unwrap();
    assert_eq!(by_name, by_value);
    assert_eq!(reg.resolve(Some(&BankRef::Resolved(Arc::clone(&by_name)))).unwrap(), by_name);

    // The process-wide registry.
    register_bank(Value::parse_json(&CUSTOM_BANK.replace("api-test-voice", "api-global-voice")).unwrap()).unwrap();
    let global = resolve_bank(Some(&"api-global-voice".into())).unwrap();
    assert!(global.phoneme("QX").is_some());
    assert!(banks().contains("api-global-voice"));
    assert_eq!(phonemes().name_str(), Some(DEFAULT_BANK));
}

#[test]
fn typed_synth_params() {
    for p in Param::ALL {
        assert_eq!(Param::from_name(p.name()), Some(p));
    }
    assert_eq!(Param::from_name("nope"), None);
    assert_eq!(Params::default(), DEFAULT);
    assert_eq!(DEFAULT[Param::F0], 120.0);

    let mut partial = PartialParams::new().with(Param::F0, 200.0);
    partial.set(Param::A1, Some(0.5));
    assert_eq!(partial.get(Param::F0), Some(200.0));
    assert_eq!(partial.get(Param::A1), Some(0.5));
    assert_eq!(partial.get(Param::F1), None);
    partial.set(Param::A1, None);
    assert_eq!(partial.get(Param::A1), None);

    let mut p = DEFAULT;
    p[Param::Gain] = 1.0;
    assert_eq!(p.0[Param::Gain as usize], 1.0);
}

#[test]
fn synth_accessors_track_state() {
    let mut synth = FormantSynth::new(1000.0, None).unwrap();
    assert_eq!(synth.sample_rate(), 1000.0);
    assert!(synth.schedule_done());

    // 10 ms at 1 kHz = 10 samples of glide.
    synth.set_target(&PartialParams::new().with(Param::F0, 220.0), 10.0);
    assert_eq!(synth.target()[Param::F0], 220.0);
    let mut buf = [0.0f32; 5];
    synth.process(&mut buf);
    assert_eq!(synth.sample_counter(), 5.0);
    assert!(synth.current()[Param::F0] > 120.0 && synth.current()[Param::F0] < 220.0);
    synth.process(&mut buf);
    assert_eq!(synth.current()[Param::F0], 220.0);

    let compiled = compile_string("AA", &CompileOptions::default()).unwrap();
    synth.queue_schedule(compiled.schedule(), 0.0);
    assert!(!synth.schedule_done());
    let mut long = vec![0.0f32; 1000];
    synth.process(&mut long);
    assert!(synth.schedule_done());
    let _ = BandpassBiquad::default();
}

#[test]
fn value_conversions() {
    assert_eq!(Value::from(1.5), Value::Num(1.5));
    assert_eq!(Value::from(true), Value::Bool(true));
    assert_eq!(Value::from(String::from("x")), Value::Str("x".into()));
    assert_eq!(Value::from(Map::new()), Value::Obj(Map::new()));

    let arr = Value::Arr(vec![Value::Num(1.0), Value::Null, Value::from("a")]);
    assert_eq!(arr.to_js_string(), "1,,a");
    assert_eq!(Value::Obj(Map::new()).to_js_string(), "[object Object]");
    assert_eq!(Value::Bool(false).to_js_string(), "false");
    assert_eq!(Value::Null.to_js_string(), "null");
    assert_eq!(Value::Arr(vec![]).to_number(), 0.0);
    assert_eq!(Value::Arr(vec![Value::Null]).to_number(), 0.0);
    assert_eq!(Value::Arr(vec![Value::Arr(vec![Value::Num(7.0)])]).to_number(), 7.0);
    assert!(Value::Arr(vec![Value::Num(1.0), Value::Num(2.0)]).to_number().is_nan());
    assert!(Value::Obj(Map::new()).to_number().is_nan());
    assert_eq!(Value::Num(1.0).as_obj(), None);
    assert_eq!(Value::Num(1.0).as_str(), None);
    assert_eq!(Value::Str("x".into()).as_f64(), None);

    let spread = arr.spread();
    assert_eq!(spread.keys().collect::<Vec<_>>(), ["0", "1", "2"]);
    assert_eq!(Value::from("a\u{1F600}").spread().len(), 3, "one key per UTF-16 unit");

    let round = Value::parse_json(r#"{"a": [1, null, {"b": "c"}], "d": true}"#).unwrap();
    assert_eq!(Value::from_json(&round.to_json()), round);
    assert_eq!(Value::Num(f64::NAN).to_json(), serde_json::Value::Null);
}

#[test]
fn utf8_dictionary_and_misc() {
    let dict = CmuDict::from_bytes("HÉLLO  HH EH1 L OW0\n".as_bytes());
    assert!(!dict.is_empty());
    assert_eq!(dict.len(), 1);
    assert_eq!(dict.entry("héllo"), Some("HH EH1 L OW0"));
    assert!(CmuDict::parse("").is_empty());

    let t = tokenize("b [OQ] b=5");
    let resets: Vec<bool> = t
        .tokens
        .iter()
        .map(|tk| match &tk.kind {
            TokenKind::Directive(d) => d.is_reset(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(resets, [true, true, false]);

    let e = Error::new("boom");
    assert_eq!(e.to_string(), "boom");
}

// V8's Math.pow special cases (v8::internal::math::pow).
#[test]
fn pow_special_cases() {
    use klattsch_core::js::pow;
    assert!(pow(2.0, f64::NAN).is_nan());
    assert!(pow(1.0, f64::INFINITY).is_nan());
    assert!(pow(-1.0, f64::NEG_INFINITY).is_nan());
    assert_eq!(pow(3.0, 2.0), 9.0);
    assert_eq!(pow(9.0, 0.5), 3.0);
    assert_eq!(pow(f64::NEG_INFINITY, 0.5), f64::INFINITY);
    assert!(pow(-0.0, 0.5).is_sign_positive());
    assert_eq!(pow(2.0, 10.0), 1024.0);
}
