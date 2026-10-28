# Step 6 checks (docs/klattsch-godot-build-spec.docx): Klattsch.save_wav
# files (compared byte for byte with klattsch-cli outside Godot, using the
# paths printed here) and KlattschSample playback. Prints one STEP6_RESULT
# JSON line and quits.
extends Node

const Verdict := preload("res://verdict.gd")
const HELLO := "HH AH L OW , W ER L D"
const CHORD := "bC3 AA r400 AA [voice=1] bE3 IY r400 IY [voice=2] bG3 UW r400 UW"

var capture: AudioEffectCapture
var finished_at := -1


func _ready() -> void:
	capture = AudioEffectCapture.new()
	capture.buffer_length = 2.0
	AudioServer.add_bus_effect(0, capture)
	var r := {"mix_rate": AudioServer.get_mix_rate(), "wavs": {}}
	DirAccess.make_dir_recursive_absolute("user://wav")

	# 1. WAV files, each with the klattsch-cli arguments that should produce
	# the same bytes.
	Klattsch.load_dictionary()
	while not Klattsch.is_dictionary_ready():
		await get_tree().process_frame
	var english := "I have $3.50"
	var japanese := "こんにちは"
	var cases := [
		["hello", Klattsch.compile(HELLO), "", 48000, [HELLO]],
		["chord", Klattsch.compile(CHORD), "", 48000, [CHORD]],
		["hello_22050", Klattsch.compile(HELLO), "", 22050, ["--sample-rate", "22050", HELLO]],
		["english", Klattsch.compile(Klattsch.english_to_phonemes(english)), english, 48000, ["--english", english]],
		["japanese", Klattsch.compile_with(Klattsch.japanese_to_phonemes(japanese), {"bank": "ja-mokhtari-2000"}), japanese, 48000,
			["--japanese", "--bank", "ja-mokhtari-2000", japanese]],
	]
	for c in cases:
		var path := "user://wav/%s.wav" % c[0]
		var err := Klattsch.save_wav(c[1], path, c[2], c[3])
		r.wavs[c[0]] = {"error": err, "path": ProjectSettings.globalize_path(path), "cli_args": c[4]}

	# 2. A KlattschSample in exact mode plays its samples untouched, then ends.
	var hello := Klattsch.render(Klattsch.compile(HELLO))
	var samples := hello.get_samples()
	r["sample_rate"] = hello.get_sample_rate()
	r["length_s"] = hello.get_length()
	r["samples"] = samples.size()
	var player := AudioStreamPlayer.new()
	add_child(player)
	player.finished.connect(func(): finished_at = Time.get_ticks_msec())
	hello.render_mode = KlattschSample.EXACT
	player.stream = hello
	var exact := await play_and_capture(player, 0.0, hello.get_length() + 0.3)
	r["exact"] = compare(exact[0], samples, 0)
	r["exact"]["finished_after_s"] = exact[1]

	# 3. Game mode at pitch 2 plays twice as fast.
	hello.render_mode = KlattschSample.GAME
	player.pitch_scale = 2.0
	var fast := await play_and_capture(player, 0.0, hello.get_length() + 0.3)
	r["pitch2_finished_after_s"] = fast[1]
	player.pitch_scale = 1.0

	# 4. Seeking: play from 0.5 s.
	hello.render_mode = KlattschSample.EXACT
	var from := 0.5
	var seeked := await play_and_capture(player, from, hello.get_length() - from + 0.3)
	r["seek"] = compare(seeked[0], samples, int(round(from * hello.get_sample_rate())))
	r["seek"]["finished_after_s"] = seeked[1]

	# 5. A stock AudioStreamWAV copy (16-bit).
	var wav := hello.to_audio_stream_wav()
	r["wav_stream"] = {"bytes": wav.data.size(), "mix_rate": wav.mix_rate, "stereo": wav.stereo, "format": wav.format}
	player.stream = wav
	var stock := await play_and_capture(player, 0.0, hello.get_length() + 0.3)
	r["wav_stream"]["finished_after_s"] = stock[1]
	# Best alignment within a few samples, then the largest difference: 16-bit
	# rounding is at most 1/65534.
	var worst := INF
	var base := first_nonzero(stock[0]) - first_nonzero(samples)
	for shift in range(-3, 4):
		var w := 0.0
		for i in range(first_nonzero(samples), samples.size()):
			var j: int = base + shift + i
			if j >= 0 and j < stock[0].size():
				w = max(w, abs(stock[0][j] - samples[i]))
		worst = min(worst, w)
	r["wav_stream"]["max_quantization_error"] = worst

	# The WAV files' bytes are compared with klattsch-cli by
	# tools/godot_tests.py; here, that they were written.
	var v := Verdict.new()
	var slack := Verdict.slack_ms() / 1000.0
	for name in r.wavs:
		v.expect("save_wav " + name, r.wavs[name].error == OK)
	var length: float = r.length_s
	v.expect("exact: identical", r.exact.identical and r.exact.compared > 0 and r.exact.tail_peak == 0.0)
	v.near("exact: finished", r.exact.finished_after_s, length, slack + 0.05)
	v.near("pitch 2: finished twice as fast", r.pitch2_finished_after_s, length / 2.0, slack + 0.05)
	v.expect("seek: identical", r.seek.identical and r.seek.compared > 0 and r.seek.tail_peak == 0.0)
	v.near("seek: finished", r.seek.finished_after_s, length - 0.5, slack + 0.05)
	v.expect("wav_stream: 16-bit mono", r.wav_stream.format == AudioStreamWAV.FORMAT_16_BITS and not r.wav_stream.stereo)
	v.expect("wav_stream: rounding only", r.wav_stream.max_quantization_error <= 1.0 / 65534.0 + 1e-9)
	v.near("wav_stream: finished", r.wav_stream.finished_after_s, length, slack + 0.05)
	v.finish(r)
	print("STEP6_RESULT ", JSON.stringify(r))
	get_tree().quit()


# Play from `from` seconds and capture the master bus (left channel) for
# `seconds`. Returns [samples, seconds until the player's `finished` signal].
func play_and_capture(player: AudioStreamPlayer, from: float, seconds: float) -> Array:
	finished_at = -1
	capture.clear_buffer()
	var out := PackedFloat32Array()
	var t0 := Time.get_ticks_msec()
	player.play(from)
	while Time.get_ticks_msec() - t0 < seconds * 1000.0:
		await get_tree().process_frame
		for f in capture.get_buffer(capture.get_frames_available()):
			out.append(f.x)
	return [out, (finished_at - t0) / 1000.0 if finished_at >= 0 else -1.0]


# Compare a capture with `reference` from index `start`, aligned at the first
# non-zero sample of each.
func compare(cap: PackedFloat32Array, reference: PackedFloat32Array, start: int) -> Dictionary:
	var ref_first := -1
	for i in range(start, reference.size()):
		if reference[i] != 0.0:
			ref_first = i
			break
	var cap_first := first_nonzero(cap)
	if ref_first < 0 or cap_first < 0:
		return {"error": "silent"}
	var mismatches := 0
	var n := reference.size() - ref_first
	for i in n:
		var j := cap_first + i
		if j >= cap.size() or cap[j] != reference[ref_first + i]:
			mismatches += 1
	var tail := 0.0
	for j in range(cap_first + n, cap.size()):
		tail = max(tail, abs(cap[j]))
	return {"compared": n, "mismatches": mismatches, "tail_peak": tail, "identical": mismatches == 0 and tail == 0.0}


func first_nonzero(s: PackedFloat32Array) -> int:
	for i in s.size():
		if s[i] != 0.0:
			return i
	return -1
