# Step 4 checks (docs/klattsch-godot-build-spec.docx): KlattschPlayer's
# say/queue/interrupt/stop behavior and the timing of its signals. Prints one
# STEP4_RESULT JSON line and quits.
#
# Signal times are compared with (a) the utterance's own phrase timings and
# (b) when its audio actually left Godot's mixer (an AudioEffectCapture on
# the master bus, timestamped each frame) plus the reported output latency.
extends Node

var capture: AudioEffectCapture
var log := []  # [usec, event, id, extra]
var captured := 0  # frames captured so far
var capture_times := []  # [cumulative frames, usec]
var first_sound := -1  # capture frame of the first non-silent sample
var listen := false


func _ready() -> void:
	process_mode = Node.PROCESS_MODE_ALWAYS  # keep testing while the tree is paused
	capture = AudioEffectCapture.new()
	capture.buffer_length = 1.0
	AudioServer.add_bus_effect(0, capture)
	var r := {"debug_build": OS.is_debug_build(), "output_latency_ms": AudioServer.get_output_latency() * 1000.0}

	var kp := KlattschPlayer.new()
	add_child(kp)
	watch(kp)
	await frames(5)
	r["internal_player"] = kp.get_audio_player().get_class()

	# 1. One utterance: signals in order, timed against phrases and the audio.
	var hello := Klattsch.compile("b140 HH AH L OW , W ER L D")
	listen = true
	first_sound = -1
	var id1 := kp.say_utterance(hello)
	await until_finished(id1)
	listen = false
	r["single"] = summarize(id1, hello)
	r["single"]["audio_vs_signal_ms"] = audio_vs_signal(id1)

	# 2. Queue: three lines back to back.
	kp.queue_mode = KlattschPlayer.QUEUE
	var a := Klattsch.compile("b140 W AH N")
	var b := Klattsch.compile("b160 T UW")
	var c := Klattsch.compile("b180 TH R IY")
	var qa := kp.say_utterance(a)
	var qb := kp.say_utterance(b)
	var qc := kp.say_utterance(c)
	await until_finished(qc)
	r["queue"] = {
		"ids": [qa, qb, qc],
		"b_start_minus_a_start_ms": t(qb, "started") - t(qa, "started"),
		"a_total_ms": a.get_total_ms(),
		"c_start_minus_b_start_ms": t(qc, "started") - t(qb, "started"),
		"b_total_ms": b.get_total_ms(),
		"order_ok": t(qa, "finished") <= t(qb, "started") + 20 and t(qb, "finished") <= t(qc, "started") + 20,
		"finished_once": count(qa, "finished") == 1 and count(qb, "finished") == 1 and count(qc, "finished") == 1,
	}

	# 3. Interrupt: a long line cut by a short one.
	kp.queue_mode = KlattschPlayer.INTERRUPT
	var long := Klattsch.compile("AA IY UW AA IY UW AA IY UW AA IY UW")
	var short := Klattsch.compile("OW K EY")
	var il := kp.say_utterance(long)
	await seconds(0.4)
	var is_ := kp.say_utterance(short)
	await until_finished(is_)
	var late_phrases := 0
	for e in log:
		if e[1] == "phrase" and e[2] == il and e[0] > when(is_, "started"):
			late_phrases += 1
	r["interrupt"] = {
		"long_finished_minus_short_started_ms": t(il, "finished") - t(is_, "started"),
		"long_phrases_after_cut": late_phrases,
		"long_finished_once": count(il, "finished") == 1,
		"long_total_ms": long.get_total_ms(),
		"long_heard_ms": t(il, "finished") - t(il, "started"),
	}

	# 4. Pausing the tree freezes the signals (and the audio). This test node
	# always processes, so make the player pausable explicitly.
	kp.process_mode = Node.PROCESS_MODE_PAUSABLE
	var paused_line := Klattsch.compile("AA IY UW AA IY UW")
	var ip := kp.say_utterance(paused_line)
	await seconds(0.3)
	get_tree().paused = true
	var pause_from := Time.get_ticks_usec()
	await seconds(0.6)
	var pause_to := Time.get_ticks_usec()
	get_tree().paused = false
	await until_finished(ip)
	var during_pause := 0
	for e in log:
		if e[2] == ip and e[0] > pause_from + 20000 and e[0] < pause_to:
			during_pause += 1
	r["pause"] = {
		"signals_during_pause": during_pause,
		"heard_ms": t(ip, "finished") - t(ip, "started"),
		"total_plus_pause_ms": paused_line.get_total_ms() + (pause_to - pause_from) / 1000.0,
	}

	# 5. stop(): every pending id finishes, at once.
	kp.queue_mode = KlattschPlayer.QUEUE
	var s1 := kp.say_utterance(long)
	var s2 := kp.say_utterance(short)
	await seconds(0.3)
	kp.stop()
	await frames(2)
	r["stop"] = {"finished": count(s1, "finished") + count(s2, "finished"), "speaking": kp.is_speaking(), "s2_started": count(s2, "started")}

	# 6. Re-entrancy: saying something from a signal handler.
	kp.queue_mode = KlattschPlayer.INTERRUPT
	var chained := [-1]
	var handler := func(id: int) -> void:
		if chained[0] == -1:
			chained[0] = kp.say_phonemes("b200 HH AY")
	var first := kp.say_phonemes("OW")
	kp.utterance_finished.connect(handler)
	await until_finished(first)
	kp.utterance_finished.disconnect(handler)
	await frames(2)
	r["reentrant_id"] = chained[0]
	if chained[0] > 0:
		await until_finished(chained[0])
		r["reentrant_finished"] = count(chained[0], "finished")

	# 7. Errors: a target that does not exist.
	var broken := KlattschPlayer.new()
	broken.target = NodePath("../Nowhere")
	add_child(broken)
	watch(broken)
	await frames(2)
	r["bad_target_id"] = broken.say_phonemes("AA")
	await frames(2)
	r["errors"] = log.filter(func(e): return e[1] == "error").map(func(e): return e[3])

	# 8. A 3D target.
	var cam := Camera3D.new()
	add_child(cam)
	cam.make_current()
	var p3 := AudioStreamPlayer3D.new()
	p3.name = "Voice3D"
	p3.position = Vector3(0, 0, -2)
	add_child(p3)
	var kp3 := KlattschPlayer.new()
	kp3.target = NodePath("../Voice3D")
	add_child(kp3)
	watch(kp3)
	await frames(3)
	var id3 := kp3.say_phonemes("HH AH L OW")
	await until_finished(id3)
	r["target_3d"] = {"player": kp3.get_audio_player().name, "stream_is_klattsch": p3.stream is KlattschStream, "finished": count(id3, "finished")}
	r["stats"] = kp.get_audio_player().stream.get_stats()

	print("STEP4_RESULT ", JSON.stringify(r))
	get_tree().quit()


func watch(kp: KlattschPlayer) -> void:
	kp.utterance_started.connect(func(id): log.append([Time.get_ticks_usec(), "started", id, null]))
	kp.phrase_started.connect(func(id, i, ph, a, b): log.append([Time.get_ticks_usec(), "phrase", id, [i, ph, a, b]]))
	kp.utterance_finished.connect(func(id): log.append([Time.get_ticks_usec(), "finished", id, null]))
	kp.error.connect(func(m): log.append([Time.get_ticks_usec(), "error", -1, m]))


func _process(_delta: float) -> void:
	var got := capture.get_buffer(capture.get_frames_available())
	if listen and first_sound < 0:
		for i in got.size():
			if got[i].x != 0.0:
				first_sound = captured + i
				break
	captured += got.size()
	capture_times.append([captured, Time.get_ticks_usec()])


func when(id: int, what: String) -> int:
	for e in log:
		if e[2] == id and e[1] == what:
			return e[0]
	return -1


func t(id: int, what: String) -> float:
	return when(id, what) / 1000.0


func count(id: int, what: String) -> int:
	return log.filter(func(e): return e[2] == id and e[1] == what).size()


# Phrase signals against the utterance's phrase timings.
func summarize(id: int, u: KlattschUtterance) -> Dictionary:
	var phrases := u.get_phrases().filter(func(p): return p.voice == 0)
	var start := when(id, "started")
	var seen := []
	var worst := 0.0
	for e in log:
		if e[2] == id and e[1] == "phrase":
			var i: int = e[3][0]
			seen.append(i)
			var late: float = (e[0] - start) / 1000.0 - phrases[i].t_start_ms
			if abs(late) > abs(worst):
				worst = late
	var in_order := true
	for k in seen.size():
		if seen[k] != k:
			in_order = false
	return {
		"phrases": phrases.size(), "phrase_signals": seen.size(), "in_order": in_order,
		"worst_phrase_offset_error_ms": worst,
		"heard_ms": t(id, "finished") - t(id, "started"), "total_ms": u.get_total_ms(),
		"started_once": count(id, "started") == 1, "finished_once": count(id, "finished") == 1,
	}


# utterance_started's time minus when its first sound left the mixer plus
# the output latency (positive: the signal came late).
func audio_vs_signal(id: int) -> float:
	if first_sound < 0:
		return NAN
	var mixed_at := -1
	for ct in capture_times:
		if ct[0] > first_sound:
			mixed_at = ct[1]
			break
	return (when(id, "started") - mixed_at) / 1000.0 - AudioServer.get_output_latency() * 1000.0


func seconds(s: float) -> void:
	await get_tree().create_timer(s, true).timeout


func frames(n: int) -> void:
	for i in n:
		await get_tree().process_frame


func until_finished(id: int) -> void:
	var limit := Time.get_ticks_msec() + 10000
	while count(id, "finished") == 0 and Time.get_ticks_msec() < limit:
		await get_tree().process_frame
