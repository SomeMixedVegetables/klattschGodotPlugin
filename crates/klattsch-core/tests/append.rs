// Tests for appending schedules (FormantSynth::append_prepared and
// Command::Append): an appended utterance starts exactly where the current
// one ends, and appending changes nothing else.

use klattsch_core::*;

const SR: f64 = 44100.0;

fn utterance(src: &str) -> (PreparedSchedule, f64) {
    let c = compile_string(src, &CompileOptions::default()).unwrap();
    (PreparedSchedule::from_schedule(SR, c.schedule()).with_duration_ms(c.total_ms), c.total_ms)
}

// Render `n` samples in uneven blocks.
fn render(synth: &mut FormantSynth, n: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; n];
    let mut at = 0;
    for size in [1, 127, 128, 512, 1000].iter().cycle() {
        if at == n {
            break;
        }
        let end = (at + size).min(n);
        synth.process(&mut out[at..end]);
        at = end;
    }
    out
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

#[test]
fn appended_utterance_starts_at_the_exact_end() {
    let (first, first_ms) = utterance("b140 HH AH L OW , W ER L D");
    let (second, second_ms) = utterance("b180 G UH D B AY");
    let end = (first_ms * SR / 1000.0).floor() as usize;
    assert_eq!(first.length(), end as f64);
    let total = end + (second_ms * SR / 1000.0) as usize + 2000;

    // Appended up front, while the first utterance is speaking.
    let mut appended = FormantSynth::new(SR, None).unwrap();
    appended.queue_prepared(first.clone(), 0.0);
    assert!(appended.speaking());
    appended.append_prepared(second.clone());
    let a = render(&mut appended, total);

    // The same thing by hand: the first utterance, then the second queued at
    // exactly its end sample.
    let mut manual = FormantSynth::new(SR, None).unwrap();
    manual.queue_prepared(first, 0.0);
    let mut b = render(&mut manual, end);
    manual.queue_prepared(second, 0.0);
    b.extend(render(&mut manual, total - end));

    assert_eq!(bits(&a), bits(&b));
    assert!(a[end..].iter().any(|x| *x != 0.0), "the second utterance is audible");
    assert!(!appended.speaking());
}

#[test]
fn append_on_a_silent_voice_starts_now() {
    let (u, ms) = utterance("AA IY UW");
    let n = (ms * SR / 1000.0) as usize + 500;
    let mut appended = FormantSynth::new(SR, None).unwrap();
    assert!(!appended.speaking());
    appended.append_prepared(u.clone());
    let mut queued = FormantSynth::new(SR, None).unwrap();
    queued.queue_prepared(u, 0.0);
    assert_eq!(bits(&render(&mut appended, n)), bits(&render(&mut queued, n)));

    // Once an utterance has ended, appending also starts at once.
    let (again, _) = utterance("OW");
    let mut late = appended.clone();
    late.append_prepared(again.clone());
    let mut now = appended.clone();
    now.queue_prepared(again, 0.0);
    assert_eq!(bits(&render(&mut late, 3000)), bits(&render(&mut now, 3000)));
}

#[test]
fn waiting_appends_are_replaced_or_cancelled() {
    let (first, _) = utterance("AA");
    let (second, _) = utterance("IY");
    let (third, _) = utterance("UW");
    let fresh = || {
        let mut s = FormantSynth::new(SR, None).unwrap();
        s.queue_prepared(first.clone(), 0.0);
        s
    };

    // Appending again replaces the waiting schedule, which is handed back.
    let mut s = fresh();
    s.append_prepared(second.clone());
    let displaced = s.append_prepared(third.clone());
    assert_eq!(displaced.len(), second.len());
    let mut only_third = fresh();
    only_third.append_prepared(third.clone());
    assert_eq!(bits(&render(&mut s, 30000)), bits(&render(&mut only_third, 30000)));

    // A new schedule cancels the waiting one; so does reset.
    let mut s = fresh();
    s.append_prepared(second.clone());
    s.queue_prepared(third.clone(), 0.0);
    let mut plain = fresh();
    plain.queue_prepared(third.clone(), 0.0);
    assert_eq!(bits(&render(&mut s, 30000)), bits(&render(&mut plain, 30000)));

    let mut s = fresh();
    s.append_prepared(second);
    s.reset(None);
    assert!(!s.speaking());
}

#[test]
fn schedules_without_a_duration_end_after_their_last_glide() {
    let c = compile_string("AA IY", &CompileOptions::default()).unwrap();
    let p = PreparedSchedule::from_schedule(SR, c.schedule());
    assert!(p.length() > 0.0 && p.length() <= (c.total_ms * SR / 1000.0).ceil() + SR);
    assert_eq!(PreparedSchedule::default().length(), 0.0);
}

#[test]
fn player_append_command() {
    let (u1, ms1) = utterance("b140 HH AH L OW");
    let (u2, _) = utterance("b200 W ER L D");
    let end = (ms1 * SR / 1000.0).floor() as usize;

    let mut direct = FormantSynth::new(SR, None).unwrap();
    direct.queue_prepared(u1.clone(), 0.0);
    direct.append_prepared(u2.clone());
    let want = render(&mut direct, end + 20000);

    let mut p = FormantProcessor::new(SR, &ProcessorOptions::default()).unwrap();
    p.handle(Command::Schedule { voice: Some(2), schedule: u1, start_time: None });
    p.handle(Command::Append { voice: Some(2), schedule: u2 });
    let mut got = Vec::new();
    let mut outs = vec![vec![0.0f32; 128]; 3];
    while got.len() < want.len() {
        let mut refs: Vec<&mut [f32]> = outs.iter_mut().map(Vec::as_mut_slice).collect();
        p.process(128, &mut refs);
        got.extend_from_slice(&outs[2]);
    }
    got.truncate(want.len());
    assert_eq!(bits(&got), bits(&want));
}

#[test]
fn tags_follow_the_playing_utterance() {
    let (a, a_ms) = utterance("AA IY");
    let (b, _) = utterance("UW");
    let end = (a_ms * SR / 1000.0).floor() as usize;
    let mut s = FormantSynth::new(SR, None).unwrap();
    assert_eq!(s.utterance_tag(), 0);
    s.queue_prepared(a.with_tag(7), -100.0);
    assert_eq!(s.utterance_tag(), 7);
    s.append_prepared(b.clone().with_tag(8));
    assert_eq!(s.utterance_tag(), 7, "an appended utterance waits");
    // Its start frame (rendered minus the schedule clock) stays put.
    render(&mut s, 1000);
    assert_eq!(1000.0 - s.sample_counter(), 100.0);
    // The first utterance's clock reaches `end` at rendered sample end + 100;
    // that sample already belongs to the second.
    render(&mut s, end + 100 - 1000);
    assert_eq!(s.utterance_tag(), 7);
    render(&mut s, 1);
    assert_eq!(s.utterance_tag(), 8, "promoted at the first one's end");
    assert_eq!((end + 101) as f64 - s.sample_counter(), (end + 100) as f64);
    s.queue_prepared(PreparedSchedule::default(), 0.0);
    assert_eq!(s.utterance_tag(), 0);
    s.queue_prepared(b.with_tag(9), 0.0);
    s.reset(None);
    assert_eq!(s.utterance_tag(), 0);
}
