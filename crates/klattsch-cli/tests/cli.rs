// Runs the built `klattsch` binary: against the real JS CLI's recorded runs
// (fixtures/cli-real.json from tools/gen-fixtures.mjs), and through the
// Rust-only options.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value as J;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("klattsch-cli-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn klattsch(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_klattsch")).args(args).current_dir(dir).output().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

#[test]
fn matches_the_js_cli() {
    let path = root().join("fixtures/cli-real.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; run `node tools/gen-fixtures.mjs`", path.display()));
    let runs: J = serde_json::from_str(&text).unwrap();
    for (i, run) in runs.as_array().unwrap().iter().enumerate() {
        let args: Vec<&str> = run["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        let dir = scratch(&format!("js{i}"));
        let out = klattsch(&dir, &args);
        let want_status = run["status"].as_i64().unwrap();
        assert_eq!(i64::from(out.status.code().unwrap()), want_status, "exit status for {args:?}");
        let want_err = run["stderr"].as_str().unwrap();
        if want_status == 0 {
            assert_eq!(stderr(&out), want_err, "stderr for {args:?}");
            let out_name = args.get(1).copied().unwrap_or("klattsch.wav");
            let got = std::fs::read(dir.join(out_name)).unwrap();
            let want = std::fs::read(root().join("fixtures/bin").join(run["wav"].as_str().unwrap())).unwrap();
            assert!(got == want, "wav bytes for {args:?}");
        } else {
            // The Rust usage text extends the JS one with the extra options.
            assert!(stderr(&out).starts_with(want_err), "usage for {args:?}: {}", stderr(&out));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn rust_only_options() {
    let dir = scratch("opts");
    let dict = root().join("data/cmudict/cmudict-0.7b");
    let dict = dict.to_str().unwrap();

    let o = klattsch(&dir, &["--english", "--dict", dict, "Hello zzqq world", "en.wav"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let e = stderr(&o);
    assert!(e.contains("not in dictionary: zzqq (sounded out)"), "{e}");
    assert!(e.contains("phonemes: b HH AH L OW' b-15 Z K W ER'-25 L D"), "{e}");
    assert!(!e.contains("warnings"), "{e}");

    // Custom word lists come before the dictionary; earlier files win.
    std::fs::write(dir.join("names.txt"), ";;; game words\nZZQQ  Z IH1 K\nHELLO  HH EY1\n").unwrap();
    std::fs::write(dir.join("more.txt"), "ZZQQ  K AA1\nWORLD  W AO1 R L D\n").unwrap();
    let words = ["--english", "--words", "names.txt", "--words", "more.txt", "--dict", dict, "Hello zzqq world", "w.wav"];
    let o = klattsch(&dir, &words);
    assert!(o.status.success(), "{}", stderr(&o));
    let e = stderr(&o);
    assert!(!e.contains("not in dictionary"), "{e}");
    assert!(e.contains("phonemes: b HH EY' b-7.5 Z IH' K b-7.5 W AO'-25 R L D"), "{e}");
    let o = klattsch(&dir, &["--english", "--words", "missing.txt", "--dict", dict, "Hi", "x.wav"]);
    assert!(!o.status.success());

    let o = klattsch(&dir, &["--english", "--no-intonation", "--dict", dict, "The FitnessGram, 20 laps.", "flat.wav"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let e = stderr(&o);
    assert!(e.contains("not in dictionary: FitnessGram = Fitness + Gram"), "{e}");
    assert!(e.contains("phonemes: DH AH F IH' T N AH S G R AE' M , T W EH' N T IY L AE' P S ."), "{e}");

    let o = klattsch(&dir, &["--japanese", "--bank", "ja-mokhtari-2000", "こんにちは", "ja.wav"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("phonemes: K O N N I CH I HH A"));
    assert!(!stderr(&o).contains("warnings"), "{}", stderr(&o));

    let o = klattsch(&dir, &["--sample-rate", "22050", "AA", "sr.wav"]);
    assert!(o.status.success());
    let wav = std::fs::read(dir.join("sr.wav")).unwrap();
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 22050);

    let o = klattsch(&dir, &["--json", "AA [voice=1] IY"]);
    assert!(o.status.success());
    let json: J = serde_json::from_slice(&o.stdout).unwrap();
    let expected =
        klattsch_core::compile_string("AA [voice=1] IY", &klattsch_core::CompileOptions::default()).unwrap();
    assert_eq!(json, expected.to_json());

    let bank = dir.join("voice.json");
    std::fs::write(
        &bank,
        r#"{"schemaVersion": 1, "name": "cli-voice", "extends": "klatt1980-en",
            "phonemes": {"QX": {"voicing": 1, "F1": 500, "F2": 1500, "F3": 2500,
                                "BW1": 60, "BW2": 90, "BW3": 120, "A1": 1, "A2": 0.8, "A3": 0.5}}}"#,
    )
    .unwrap();
    let o = klattsch(&dir, &["--load-bank", "voice.json", "--bank", "cli-voice", "QX AA", "bank.wav"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("loaded bank cli-voice from voice.json"));
    assert!(!stderr(&o).contains("warnings"), "{}", stderr(&o));

    for (args, msg) in [
        (&["--bogus", "AA"][..], "unknown option: --bogus"),
        (&["--sample-rate", "0", "AA"][..], "bad sample rate: 0"),
        (&["--sample-rate"][..], "--sample-rate needs a value"),
        (&["--load-bank", "missing.json", "AA"][..], "cannot read bank missing.json"),
        (&["--bank", "nope", "AA"][..], "unknown bank: nope"),
        (&["--english", "--dict", "missing.dict", "hi"][..], "cannot read dictionary missing.dict"),
        (&["--bank"][..], "--bank needs a value"),
        (&["--load-bank"][..], "--load-bank needs a value"),
        (&["--dict"][..], "--dict needs a value"),
        (&["--help"][..], "usage: klattsch"),
        (&["AA", "no-such-dir/out.wav"][..], "cannot write no-such-dir"),
        (&["r-99999 AA", "neg.wav"][..], "Invalid typed array length: -"),
    ] {
        let o = klattsch(&dir, args);
        assert_eq!(o.status.code(), Some(1), "{args:?}");
        assert!(stderr(&o).contains(msg), "{args:?}: {}", stderr(&o));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
