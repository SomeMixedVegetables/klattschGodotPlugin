# Step 3 check (docs/klattsch-godot-build-spec.docx): what an exact-mode
# KlattschStream sends to the master bus is identical, sample for sample, to
# klattsch's offline render (KlattschUtterance.render) at the same rate.
# Prints one STEP3_RESULT JSON line and quits.
#
# Each case starts a fresh playback (so the synths start from rest, like the
# offline render), records the master bus with an AudioEffectCapture (the
# bus's only effect, volume 0 dB) while speaking, aligns the capture to the
# reference at their first non-zero samples, and compares both channels bit
# for bit.
extends Node

const GAME := 0
const EXACT := 1

var capture: AudioEffectCapture
var rate: float


func _ready() -> void:
	capture = AudioEffectCapture.new()
	capture.buffer_length = 2.0
	AudioServer.add_bus_effect(0, capture)
	rate = AudioServer.get_mix_rate()
	var r := {"mix_rate": rate, "debug_build": OS.is_debug_build(), "bus_volume_db": AudioServer.get_bus_volume_db(0), "cases": []}

	var hello := Klattsch.compile("b140 HH AH L OW , W ER L D")
	var chord := Klattsch.compile("bC3 AA r400 AA [voice=1] bE3 IY r400 IY [voice=2] bG3 UW r400 UW")
	var styled := Klattsch.compile_with("S IY K W AH N S , TH R IY TH AW Z AH N D",
		{"base_f0": 95.0, "rate": 1.3, "vibrato_depth": 4.0, "tremolo_depth": 0.2, "aspiration": 0.1, "tilt": 0.3, "effort": 0.7})

	var flat := AudioStreamPlayer.new()
	add_child(flat)
	r.cases.append(await compare("hello, AudioStreamPlayer", flat, EXACT, hello))
	r.cases.append(await compare("3-voice chord, AudioStreamPlayer", flat, EXACT, chord))
	r.cases.append(await compare("voice options, AudioStreamPlayer", flat, EXACT, styled))

	# Positional players, centered on the listener: does Godot apply a fixed
	# gain (pan law) to centered sound?
	var cam := Camera2D.new()
	add_child(cam)
	cam.make_current()
	var p2 := AudioStreamPlayer2D.new()
	add_child(p2)
	r.cases.append(await compare("hello, AudioStreamPlayer2D at the listener", p2, EXACT, hello))
	var cam3 := Camera3D.new()
	add_child(cam3)
	cam3.make_current()
	var p3 := AudioStreamPlayer3D.new()
	add_child(p3)
	p3.position = Vector3(0, 0, -1)  # straight ahead at unit_size: 0 dB distance attenuation
	r.cases.append(await compare("hello, AudioStreamPlayer3D 1 m ahead", p3, EXACT, hello))
	p3.attenuation_filter_cutoff_hz = 20500.0  # the 3D low-pass filter off
	r.cases.append(await compare("hello, AudioStreamPlayer3D 1 m ahead, no filter", p3, EXACT, hello))

	# For comparison: game mode at the mix rate, pitch 1 (resampled by Godot).
	r.cases.append(await compare("hello, game mode", flat, GAME, hello))

	print("STEP3_RESULT ", JSON.stringify(r))
	get_tree().quit()


func compare(name: String, player: Node, mode: int, u: KlattschUtterance) -> Dictionary:
	var stream := KlattschStream.new()
	stream.render_mode = mode
	player.stream = stream
	player.play()
	await get_tree().create_timer(0.15).timeout
	var reference := u.render()
	capture.clear_buffer()
	stream.speak(u)
	var left := PackedFloat32Array()
	var right := PackedFloat32Array()
	var need := reference.size() + int(rate * 0.3)
	while left.size() < need:
		await get_tree().process_frame
		for f in capture.get_buffer(capture.get_frames_available()):
			left.append(f.x)
			right.append(f.y)
	player.stop()

	var res := {"name": name, "mode": "exact" if mode == EXACT else "game", "samples": reference.size()}
	var ref_start := first_nonzero(reference)
	var cap_start := first_nonzero(left)
	res["silent_before"] = cap_start
	if cap_start < 0 or ref_start < 0:
		res["error"] = "silent capture"
		return res
	var offset := cap_start - ref_start
	res["offset"] = offset
	var mismatches := 0
	var right_mismatches := 0
	var max_diff := 0.0
	var first_mismatch := -1
	var ratio_sum := 0.0
	var ratio_n := 0
	for i in reference.size():
		var j := offset + i
		var want := reference[i]
		var got := left[j] if j >= 0 and j < left.size() else NAN
		if got != want:
			mismatches += 1
			if first_mismatch < 0:
				first_mismatch = i
			max_diff = max(max_diff, abs(got - want))
		if j >= 0 and j < right.size() and right[j] != left[j]:
			right_mismatches += 1
		if abs(want) > 0.01:
			ratio_sum += got / want
			ratio_n += 1
	# Whatever follows the utterance must be silence.
	var tail_peak := 0.0
	for j in range(offset + reference.size(), left.size()):
		tail_peak = max(tail_peak, abs(left[j]))
	res["mismatches"] = mismatches
	res["right_differs_from_left"] = right_mismatches
	res["first_mismatch"] = first_mismatch
	res["max_abs_diff"] = max_diff
	res["mean_gain"] = ratio_sum / ratio_n if ratio_n > 0 else 0.0
	res["tail_peak"] = tail_peak
	res["identical"] = mismatches == 0 and right_mismatches == 0 and tail_peak == 0.0
	# Does the capture equal the reference times one fixed gain (computed in
	# float32, as Godot mixes)? Report that gain and the worst sample error.
	var gain := 0.0
	for i in reference.size():
		if abs(reference[i]) > 0.1 and offset + i < left.size():
			gain = left[offset + i] / reference[i]
			break
	var scaled_mismatches := 0
	var worst := 0.0
	var g32 := PackedFloat32Array([gain])
	for i in reference.size():
		var j := offset + i
		if j >= left.size():
			break
		var scaled := PackedFloat32Array([reference[i] * g32[0]])[0]
		if left[j] != scaled:
			scaled_mismatches += 1
			worst = max(worst, abs(left[j] - scaled))
	res["fixed_gain"] = gain
	res["fixed_gain_mismatches"] = scaled_mismatches
	res["fixed_gain_worst_diff"] = worst
	return res


func first_nonzero(s: PackedFloat32Array) -> int:
	for i in s.size():
		if s[i] != 0.0:
			return i
	return -1
