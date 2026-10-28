// Helpers shared by the parity test suites.
#![allow(dead_code)]

use std::collections::HashSet;
use std::path::PathBuf;

use klattsch_core::Phone;
use serde_json::Value as J;

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn load(name: &str) -> J {
    let path = fixtures_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; run `node tools/gen-fixtures.mjs`", path.display()));
    serde_json::from_str(&text).unwrap()
}

pub fn bin(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join("bin").join(name)).unwrap()
}

pub fn f32s(name: &str) -> Vec<f32> {
    bin(name).as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

pub fn num(v: &J) -> f64 {
    v.as_f64().unwrap()
}

// Structural JSON equality; numbers compare by exact f64 value.
pub fn json_diff(a: &J, b: &J, path: &str) -> Option<String> {
    match (a, b) {
        (J::Number(x), J::Number(y)) => {
            (x.as_f64() != y.as_f64()).then(|| format!("{path}: {x} != {y}"))
        }
        (J::Array(x), J::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} != {}", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| json_diff(p, q, &format!("{path}[{i}]")))
        }
        (J::Object(x), J::Object(y)) => {
            let xk: HashSet<_> = x.keys().collect();
            let yk: HashSet<_> = y.keys().collect();
            if xk != yk {
                return Some(format!("{path}: keys {:?} != {:?}", xk, yk));
            }
            x.iter().find_map(|(k, v)| json_diff(v, &y[k], &format!("{path}.{k}")))
        }
        _ => (a != b).then(|| format!("{path}: {a} != {b}")),
    }
}

#[track_caller]
pub fn assert_json(actual: &J, expected: &J, ctx: &str) {
    if let Some(d) = json_diff(actual, expected, "$") {
        panic!("{ctx}: {d}");
    }
}

#[track_caller]
pub fn assert_samples(actual: &[f32], expected: &[f32], ctx: &str) {
    assert_eq!(actual.len(), expected.len(), "{ctx}: sample count");
    let bad: Vec<usize> =
        (0..actual.len()).filter(|&i| actual[i].to_bits() != expected[i].to_bits()).collect();
    if let Some(&first) = bad.first() {
        let max = bad.iter().map(|&i| (actual[i] - expected[i]).abs()).fold(0.0f32, f32::max);
        panic!(
            "{ctx}: {} of {} samples differ; first at {first}: {} vs {}; max abs diff {max:e}",
            bad.len(),
            actual.len(),
            actual[first],
            expected[first],
        );
    }
}

pub fn to_value(v: &J) -> klattsch_core::Value {
    klattsch_core::Value::from_json(v)
}

pub fn phones_json(p: &Option<Vec<Phone>>) -> J {
    match p {
        Some(v) => J::Array(v.iter().map(Phone::to_json).collect()),
        None => J::Null,
    }
}

// --- numbers and values as the fixture generators write them -----------------
// Non-finite numbers are tagged {"$f": "NaN"} (JSON has no NaN); "-0" is -0.

pub fn tagged_num(v: &J) -> f64 {
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

pub fn tagged_value(v: &J) -> klattsch_core::Value {
    use klattsch_core::Value;
    match v {
        J::Object(o) if o.len() == 1 && o.contains_key("$f") => Value::Num(tagged_num(v)),
        J::Object(o) => Value::Obj(o.iter().map(|(k, x)| (k.clone(), tagged_value(x))).collect()),
        J::Array(a) => Value::Arr(a.iter().map(tagged_value).collect()),
        other => Value::from_json(other),
    }
}

// --- the AudioWorklet replay (see tools/worklet-harness.mjs) ----------------

pub const QUANTUM: usize = 128;

fn partial(o: &J) -> klattsch_core::PartialParams {
    klattsch_core::PartialParams::from_map(
        &o.as_object().unwrap().iter().map(|(k, v)| (k.clone(), tagged_value(v))).collect(),
    )
}

fn synth_events(events: &J) -> Vec<klattsch_core::SynthEvent> {
    events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| klattsch_core::SynthEvent {
            at_ms: e.get("atMs").map(tagged_num),
            target: partial(&e["target"]),
            transition_ms: e.get("transitionMs").map(tagged_num),
        })
        .collect()
}

// A worklet `port` message as a Command; None for messages the worklet ignores.
pub fn worklet_command(d: &J, sample_rate: f64) -> Option<klattsch_core::Command> {
    use klattsch_core::{Command, PreparedSchedule};
    let voice = match d.get("voice") {
        None | Some(J::Null) => None,
        Some(v) => Some(v.as_u64().unwrap() as usize),
    };
    let optional = |k: &str| d.get(k).filter(|x| !x.is_null());
    match d.get("type").and_then(J::as_str) {
        Some("frame") => Some(Command::Frame {
            voice,
            target: partial(&d["target"]),
            // `setTarget(target, transitionMs = 30)`: undefined takes the
            // default, null becomes 0.
            transition_ms: match d.get("transitionMs") {
                None => 30.0,
                Some(J::Null) => 0.0,
                Some(x) => tagged_num(x),
            },
        }),
        Some("schedule") => Some(Command::Schedule {
            voice,
            schedule: PreparedSchedule::from_events(sample_rate, &synth_events(&d["schedule"])),
            start_time: optional("startTime").map(tagged_num),
        }),
        Some("reset") => Some(Command::Reset { voice, initial_target: optional("initialTarget").map(partial) }),
        _ => None,
    }
}

pub fn worklet_options(po: &J) -> klattsch_core::ProcessorOptions {
    klattsch_core::ProcessorOptions {
        schedule: po.get("schedule").map(synth_events),
        initial_target: po.get("initialTarget").map(partial),
        schedules: po.get("schedules").and_then(J::as_array).map_or_else(Vec::new, |a| {
            a.iter().map(|s| if s.is_null() { None } else { Some(synth_events(s)) }).collect()
        }),
    }
}

// Replays a worklet scenario; per output, channel 0's samples (None for an
// output with no channels).
pub fn run_worklet(sc: &J) -> Vec<Option<Vec<f32>>> {
    let sr = tagged_num(&sc["sr"]);
    let channels: Vec<u64> = sc["outputs"].as_array().unwrap().iter().map(|c| c.as_u64().unwrap()).collect();
    let mut p = klattsch_core::FormantProcessor::new(sr, &worklet_options(&sc["processorOptions"])).unwrap();
    let mut recorded: Vec<Vec<f32>> = vec![Vec::new(); channels.len()];
    for op in sc["ops"].as_array().unwrap() {
        if op["op"] == "message" {
            if let Some(cmd) = worklet_command(&op["data"], sr) {
                p.handle(cmd);
            }
            continue;
        }
        for _ in 0..op["quanta"].as_u64().unwrap() {
            // Dirty buffers: the processor must overwrite or zero every sample.
            let mut bufs: Vec<Vec<f32>> =
                channels.iter().map(|&c| if c == 0 { Vec::new() } else { vec![f32::NAN; QUANTUM] }).collect();
            let mut outs: Vec<&mut [f32]> = bufs.iter_mut().map(Vec::as_mut_slice).collect();
            p.process(QUANTUM, &mut outs);
            for (v, b) in bufs.into_iter().enumerate() {
                recorded[v].extend(b);
            }
        }
    }
    recorded.into_iter().zip(&channels).map(|(r, &c)| (c > 0).then_some(r)).collect()
}
