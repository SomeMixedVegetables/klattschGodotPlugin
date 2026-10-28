// Tests for the threaded Player / PlayerHandle split and phrase lookup. The
// processor itself is checked against the JS AudioWorklet in parity.rs and
// fuzz_parity.rs.

use klattsch_core::*;

const BLOCK: usize = 256;

fn hello() -> CompileResult {
    compile_string("HH AH L OW , W ER L D", &CompileOptions::default()).unwrap()
}

fn render(player: &mut Player, blocks: usize, voices: usize) -> Vec<f32> {
    let mut out = Vec::new();
    let mut block = vec![0.0f32; BLOCK];
    for _ in 0..blocks {
        player.process_mix(&mut block, voices);
        out.extend_from_slice(&block);
    }
    out
}

#[test]
fn handle_commands_match_direct_processor_use() {
    let sr = 48000.0;
    let compiled = hello();
    let (handle, mut player) = player(sr, ProcessorOptions::default(), 16).unwrap();
    let mut direct = FormantProcessor::new(sr, &ProcessorOptions::default()).unwrap();

    let mut via_player = Vec::new();
    let mut via_direct = Vec::new();
    let mut a = vec![0.0f32; BLOCK];
    let mut b = vec![0.0f32; BLOCK];
    for block in 0..200 {
        // The same commands, applied at the same block boundaries.
        let cmds: Vec<Command> = match block {
            3 => vec![Command::Schedule {
                voice: Some(1),
                schedule: PreparedSchedule::from_schedule(sr, compiled.schedule()),
                start_time: None,
            }],
            50 => vec![
                Command::Frame { voice: Some(1), target: PartialParams::new().with(Param::F0, 180.0), transition_ms: 40.0 },
                Command::Schedule {
                    voice: None,
                    schedule: PreparedSchedule::from_schedule(sr, compiled.schedule()),
                    start_time: Some(0.1),
                },
            ],
            120 => vec![Command::Reset { voice: None, initial_target: None }],
            _ => vec![],
        };
        for cmd in cmds {
            direct.handle(cmd.clone());
            handle.send(cmd).unwrap();
        }
        player.process(BLOCK, &mut [&mut a[..], &mut b[..]]);
        via_player.extend_from_slice(&a);
        via_player.extend_from_slice(&b);
        let (mut c, mut d) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
        direct.process(BLOCK, &mut [&mut c[..], &mut d[..]]);
        via_direct.extend_from_slice(&c);
        via_direct.extend_from_slice(&d);
    }
    assert!(via_player.iter().zip(&via_direct).all(|(x, y)| x.to_bits() == y.to_bits()));
    assert!(via_player.iter().any(|x| *x != 0.0), "something was spoken");
    assert_eq!(handle.current_time(), 200.0 * BLOCK as f64 / sr);
}

#[test]
fn mix_is_the_sum_of_the_voices() {
    let sr = 44100.0;
    let chord = compile_string("bC3 AA r400 AA [voice=1] bE3 IY r400 IY [voice=2] bG3 UW r400 UW", &CompileOptions::default())
        .unwrap();
    let schedules: Vec<Option<Vec<SynthEvent>>> =
        chord.voices.iter().map(|v| Some(v.schedule.iter().map(SynthEvent::from).collect())).collect();
    let opts = ProcessorOptions { schedules, ..Default::default() };

    let mut split = FormantProcessor::new(sr, &opts).unwrap();
    let mut mixed = FormantProcessor::new(sr, &opts).unwrap();
    let mut scratch = vec![0.0f32; 5000];
    for len in [128, 1, 777, 4096, 128, 3000] {
        let mut voices = vec![vec![0.0f32; len]; 3];
        let mut outs: Vec<&mut [f32]> = voices.iter_mut().map(Vec::as_mut_slice).collect();
        split.process(len, &mut outs);
        let mut out = vec![1.0f32; len];
        mixed.process_mix(&mut out, 3, &mut scratch);
        for i in 0..len {
            let sum = voices[0][i] + voices[1][i] + voices[2][i];
            assert_eq!(out[i].to_bits(), sum.to_bits(), "sample {i} of block {len}");
        }
    }
    assert_eq!(split.frames(), mixed.frames());

    // Player::process_mix renders in chunks; the result is the same.
    let (_, mut player) = player(sr, opts.clone(), 4).unwrap();
    let mut big = vec![0.0f32; 10_000];
    player.process_mix(&mut big, 3);
    let mut reference = FormantProcessor::new(sr, &opts).unwrap();
    let mut whole = vec![0.0f32; 10_000];
    reference.process_mix(&mut whole, 3, &mut vec![0.0; 10_000]);
    assert!(big.iter().zip(&whole).all(|(x, y)| x.to_bits() == y.to_bits()));
}

#[test]
fn full_queue_hands_the_command_back() {
    let (handle, mut player) = player(48000.0, ProcessorOptions::default(), 2).unwrap();
    let target = PartialParams::new().with(Param::F0, 200.0);
    handle.frame(None, target, None).unwrap();
    handle.frame(None, target, Some(10.0)).unwrap();
    let rejected = handle.frame(Some(3), target, None).unwrap_err();
    assert_eq!(rejected, Command::Frame { voice: Some(3), target, transition_ms: 30.0 });
    render(&mut player, 1, 1);
    handle.send(rejected).unwrap();
}

#[test]
fn replaced_schedules_are_recycled() {
    // Far more schedule swaps than the return queue holds: the handle frees
    // them as it sends, so nothing blocks or fails.
    let compiled = hello();
    let (handle, mut player) = player(22050.0, ProcessorOptions::default(), 4).unwrap();
    for i in 0..500 {
        handle.schedule(Some(i % 3), compiled.schedule(), Some(handle.current_time())).unwrap();
        render(&mut player, 1, 3);
    }
    handle.collect_garbage();
    assert!(player.processor().voice(2).is_some());
}

#[test]
fn player_runs_on_another_thread() {
    let compiled = hello();
    let (handle, mut player) = player(48000.0, ProcessorOptions::default(), 64).unwrap();
    handle.schedule(None, compiled.schedule(), None).unwrap();
    let audio = std::thread::spawn(move || render(&mut player, 400, 1));
    let samples = audio.join().unwrap();
    assert!(samples.iter().any(|x| *x != 0.0));
    assert_eq!(handle.current_time(), 400.0 * BLOCK as f64 / 48000.0);
    let target = PartialParams::new().with(Param::F0, 100.0);
    assert!(handle.frame(None, target, None).is_err(), "player dropped: queue disconnected");
}

#[test]
fn bad_sample_rate_is_rejected() {
    assert!(player(0.0, ProcessorOptions::default(), 4).is_err());
    assert!(FormantProcessor::new(f64::NAN, &ProcessorOptions::default()).is_err());
}

#[test]
fn phrase_lookup() {
    let compiled = hello();
    let phrases = compiled.phrases();
    assert_eq!(phrase_at(phrases, -1.0), None);
    assert_eq!(phrase_at(phrases, 0.0), Some(0));
    assert_eq!(phrases[0].phoneme.as_deref(), Some("HH"));
    for (i, p) in phrases.iter().enumerate() {
        if p.t_end_ms > p.t_start_ms {
            assert_eq!(phrase_at(phrases, p.t_start_ms), Some(i));
            assert_eq!(phrase_at(phrases, (p.t_start_ms + p.t_end_ms) / 2.0), Some(i));
        }
    }
    assert_eq!(phrase_at(phrases, compiled.total_ms), None);
    assert_eq!(phrase_at(&[], 5.0), None);
}

#[test]
fn dropped_handle_and_accessors() {
    let compiled = hello();
    let (handle, mut player) = player(32000.0, ProcessorOptions::default(), 4).unwrap();
    assert_eq!(handle.sample_rate(), 32000.0);
    assert_eq!(player.processor().sample_rate(), 32000.0);
    let prepared = PreparedSchedule::from_schedule(32000.0, compiled.schedule());
    assert_eq!(prepared.len(), compiled.schedule().len());
    assert!(!prepared.is_empty() && PreparedSchedule::default().is_empty());
    assert_eq!(prepared.sample_rate(), 32000.0);

    // Two schedules replace two old ones; with the handle gone, the player
    // frees them itself instead of returning them.
    handle.schedule(Some(0), compiled.schedule(), None).unwrap();
    handle.schedule(Some(0), compiled.schedule(), None).unwrap();
    drop(handle);
    let out = render(&mut player, 50, 1);
    assert!(out.iter().any(|x| *x != 0.0));
    assert!(player.processor().voice(0).is_some_and(|v| v.sample_counter() > 0.0));
}
