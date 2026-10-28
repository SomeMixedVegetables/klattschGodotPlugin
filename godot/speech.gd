# Step 2 checks (docs/klattsch-godot-build-spec.docx): klattsch speech from
# KlattschStream, appending, live parameter glides, silence and reset.
# Prints one STEP2_RESULT JSON line and quits.
extends Node3D

const SILENT := 0.0005  # |sample| below this counts as silence

@onready var flat: AudioStreamPlayer = $Flat
@onready var spatial: AudioStreamPlayer3D = $Spatial

var capture: AudioEffectCapture
var rate: float


func _ready() -> void:
	capture = AudioEffectCapture.new()
	capture.buffer_length = 1.0
	AudioServer.add_bus_effect(0, capture)
	rate = AudioServer.get_mix_rate()
	var r := {"mix_rate": rate, "debug_build": OS.is_debug_build()}

	var stream := KlattschStream.new()
	stream.render_mode = KlattschStream.EXACT
	var hello := Klattsch.compile("b140 HH AH L OW , W ER L D")
	r["speak_before_play"] = stream.speak(hello)  # expected false
	r["local_to_scene"] = stream.resource_local_to_scene
	flat.stream = stream
	flat.play()
	await seconds(0.2)

	# 1. One utterance: audible for about total_ms.
	r["hello_total_ms"] = hello.get_total_ms()
	r["hello_phrases"] = hello.get_phrases().size()
	r["speak"] = stream.speak(hello)
	r["hello_span_ms"] = span_ms(await record(hello.get_total_ms() / 1000.0 + 0.6))

	# 2. Append: the second line starts where the first ends.
	var first := Klattsch.compile("b140 HH AH L OW")
	var second := Klattsch.compile("b190 G UH D B AY")
	r["first_ms"] = first.get_total_ms()
	r["second_ms"] = second.get_total_ms()
	stream.speak(first)
	r["append"] = stream.speak(second, false)
	r["append_span_ms"] = span_ms(await record((first.get_total_ms() + second.get_total_ms()) / 1000.0 + 0.6))

	# 3. Live control only: a sustained vowel built with set_params, then an
	# F0 glide, measured from the captured audio.
	stream.set_params({"voicing": 1.0, "F1": 700.0, "F2": 1220.0, "F3": 2600.0,
		"A1": 1.0, "A2": 0.6, "A3": 0.3, "F0": 120.0}, 30.0, 0)
	r["vowel_f0_before"] = f0(await record(0.5))
	r["set_param"] = stream.set_param(&"F0", 220.0, 0.0, 0)
	await record(0.1)
	r["vowel_f0_after"] = f0(await record(0.4))
	r["bad_param"] = stream.set_param(&"nope", 1.0)  # expected false (and an error)

	# 4. silence(): quiet within a short fade.
	stream.silence()
	await record(0.15)
	r["after_silence_peak"] = peak(await record(0.3))

	# 5. reset(): a voice that was sounding goes back to the silent defaults.
	stream.set_params({"voicing": 1.0, "A1": 1.0, "F1": 500.0}, 10.0, 1)
	await record(0.2)
	r["reset"] = stream.reset()
	await record(0.05)
	r["after_reset_peak"] = peak(await record(0.3))

	# 6. Interrupting: a long line cut short by another.
	var long := Klattsch.compile("AA IY UW AA IY UW AA IY UW AA IY UW AA IY UW")
	var short := Klattsch.compile("OW")
	stream.speak(long)
	await seconds(0.3)
	stream.speak(short)
	r["interrupt_span_ms"] = span_ms(await record(1.5))
	r["long_ms"] = long.get_total_ms()
	r["short_ms"] = short.get_total_ms()

	r["exact_stats"] = stream.get_stats()
	r["audio_time"] = stream.get_audio_time()
	flat.stop()

	# 7. Game mode on a 3D player.
	var game := KlattschStream.new()
	spatial.stream = game
	spatial.play()
	await seconds(0.1)
	game.speak(hello)
	r["game_span_ms"] = span_ms(await record(hello.get_total_ms() / 1000.0 + 0.6))
	r["game_stats"] = game.get_stats()
	spatial.stop()
	spatial.stream = null
	flat.stream = null
	await seconds(0.1)
	r["after_stop"] = KlattschStream.new().get_stats()
	print("STEP2_RESULT ", JSON.stringify(r))
	get_tree().quit()


func seconds(t: float) -> void:
	await get_tree().create_timer(t).timeout


# Capture the master bus (left channel) for `t` seconds.
func record(t: float) -> PackedFloat32Array:
	capture.clear_buffer()
	var out := PackedFloat32Array()
	var elapsed := 0.0
	while elapsed < t:
		await get_tree().process_frame
		elapsed += get_process_delta_time()
		for f in capture.get_buffer(capture.get_frames_available()):
			out.append(f.x)
	return out


# Milliseconds from the first to the last non-silent sample.
func span_ms(s: PackedFloat32Array) -> float:
	var first := -1
	var last := -1
	for i in s.size():
		if abs(s[i]) > SILENT:
			if first < 0:
				first = i
			last = i
	return 0.0 if first < 0 else (last - first) / rate * 1000.0


func peak(s: PackedFloat32Array) -> float:
	var m := 0.0
	for x in s:
		m = max(m, abs(x))
	return m


# Fundamental frequency by autocorrelation over 60-400 Hz.
func f0(s: PackedFloat32Array) -> float:
	var n: int = min(s.size(), 4096)
	var start: int = s.size() - n
	var best_lag := 0
	var best := -INF
	for lag in range(int(rate / 400.0), int(rate / 60.0)):
		var acc := 0.0
		for i in range(start, start + n - lag):
			acc += s[i] * s[i + lag]
		if acc > best:
			best = acc
			best_lag = lag
	return rate / best_lag if best_lag > 0 else 0.0
