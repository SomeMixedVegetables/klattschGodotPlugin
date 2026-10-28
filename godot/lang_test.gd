# Step 5 checks (docs/klattsch-godot-build-spec.docx): the Klattsch
# singleton (background dictionary, lexicon, word lists, banks through
# FileAccess, English and Japanese) and KlattschPlayer.say / say_japanese.
# Works in the editor and in an exported build. Prints one STEP5_RESULT JSON
# line and quits.
extends Node

# klattsch-cli --english output for the same text (must match exactly).
const CLI := {
	"I have $3.50": "b AY HH AE V TH R IY' D AA' L ER Z AH N D F IH' F T IY S EH'-25 N T S",
	"Dr. Smith paid $4.50 at 3:30 p.m.": "b D AA' K T ER b-1.67 S M IH' TH b-1.67 P EY' D b-1.67 F AO' R D AA' L ER Z AH N D F IH' F T IY S EH' N T S b-6.67 AE T TH R IY' TH ER' D IY P IY EH'-25 M .",
	"Where are you going?": "b W EH' R b-15 AA R Y UW G OW'-25 IH NG .",
}

const BANK := """{"schemaVersion": 1, "name": "user-voice", "extends": "klatt1980-en",
	"phonemes": {"QX": {"voicing": 1, "F1": 500, "F2": 1500, "F3": 2500,
	"BW1": 60, "BW2": 90, "BW3": 120, "A1": 1, "A2": 0.8, "A3": 0.5}}}"""

var log := []
var capture: AudioEffectCapture


func _ready() -> void:
	capture = AudioEffectCapture.new()
	capture.buffer_length = 1.0
	AudioServer.add_bus_effect(0, capture)
	var r := {"exported": not OS.has_feature("editor"), "debug_build": OS.is_debug_build()}

	var kp := KlattschPlayer.new()
	add_child(kp)
	kp.utterance_finished.connect(func(id): log.append(["finished", id]))
	kp.error.connect(func(m): log.append(["error", m]))
	var loaded := [false]
	Klattsch.dictionary_loaded.connect(func(): loaded[0] = true)
	await get_tree().process_frame

	# 1. Before the dictionary: say() starts the load, still speaks, and
	# names the words it had to sound out.
	r["ready_before"] = Klattsch.is_dictionary_ready()
	var t0 := Time.get_ticks_msec()
	var early := kp.say("Hello zorblax")
	r["early_id"] = early
	r["early_errors"] = errors()
	while not loaded[0] and Time.get_ticks_msec() - t0 < 10000:
		await get_tree().process_frame
	r["dictionary_loaded_signal"] = loaded[0]
	r["dictionary_ms"] = Time.get_ticks_msec() - t0
	r["ready_after"] = Klattsch.is_dictionary_ready()
	await until_finished(kp, early)

	# 2. English matches klattsch-cli exactly.
	var english := {}
	for text in CLI:
		english[text] = Klattsch.english_to_phonemes(text) == CLI[text]
	r["english_matches_cli"] = english
	r["english_flat"] = Klattsch.english_to_phonemes_with("Is it?", {"intonation": false})
	r["words"] = Klattsch.english_words("Dr. Klattsch 99")

	# 3. Lexicon: add_word, and a word list read from user://.
	r["add_word"] = Klattsch.add_word("Zyx", "Z IH1 K S")
	r["add_word_bad"] = Klattsch.add_word("Zyx", "NOT PHONES 123")
	write("user://words.txt", ";;; test words\nKLATTSCH  K L AA1 CH\nZYX  W AA1 T\n")
	r["load_word_list"] = Klattsch.load_word_list("user://words.txt")
	r["zyx"] = Klattsch.english_to_phonemes_with("Zyx", {"intonation": false})  # add_word wins: Z IH' K S
	r["klattsch_word"] = Klattsch.english_to_phonemes_with("Klattsch", {"intonation": false})
	r["remove_word"] = Klattsch.remove_word("Zyx")
	r["zyx_after_remove"] = Klattsch.english_to_phonemes_with("Zyx", {"intonation": false})

	# 4. A custom bank from user:// speaks.
	write("user://voice.json", BANK)
	r["load_bank"] = Klattsch.load_bank("user://voice.json")
	r["banks"] = Klattsch.get_bank_names()
	r["qx_warnings_default_bank"] = Klattsch.compile("QX AA").get_warnings().size()
	r["qx_warnings_user_bank"] = Klattsch.compile_with("QX AA", {"bank": "user-voice"}).get_warnings().size()
	kp.stream.bank = "user-voice"
	var qx := kp.say_phonemes("QX AA")
	capture.clear_buffer()
	await until_finished(kp, qx)
	r["user_bank_peak"] = peak(drain())
	kp.stream.bank = ""

	# 5. Voice properties reach say(): base_f0 200 on a plain vowel.
	kp.stream.base_f0 = 200.0
	kp.stream.rate = 600.0
	var vowel := kp.say_phonemes("AA")
	await get_tree().create_timer(0.15).timeout
	capture.clear_buffer()
	await get_tree().create_timer(0.3).timeout
	r["f0_at_base_200"] = f0(drain())
	await until_finished(kp, vowel)
	kp.stream.base_f0 = 120.0
	kp.stream.rate = 110.0

	# 6. Japanese.
	r["japanese"] = Klattsch.japanese_to_phonemes("こんにちは")
	r["japanese_bad"] = Klattsch.japanese_to_phonemes("123")
	var ja := kp.say_japanese("こんにちは")
	await until_finished(kp, ja)
	r["say_japanese_id"] = ja
	r["say_japanese_bad"] = kp.say_japanese("123")

	# 7. The spec's acceptance line.
	var money := kp.say("I have $3.50")
	await until_finished(kp, money)
	r["say_money_id"] = money
	r["all_errors"] = errors()
	kp.get_audio_player().stop()
	await get_tree().process_frame
	print("STEP5_RESULT ", JSON.stringify(r))
	get_tree().quit()


func errors() -> Array:
	return log.filter(func(e): return e[0] == "error").map(func(e): return e[1])


func until_finished(kp: KlattschPlayer, id: int) -> void:
	if id < 0:
		return
	var limit := Time.get_ticks_msec() + 10000
	while not log.has(["finished", id]) and Time.get_ticks_msec() < limit:
		await get_tree().process_frame


func write(path: String, text: String) -> void:
	var f := FileAccess.open(path, FileAccess.WRITE)
	f.store_string(text)
	f.close()


func drain() -> PackedFloat32Array:
	var out := PackedFloat32Array()
	for f in capture.get_buffer(capture.get_frames_available()):
		out.append(f.x)
	return out


func peak(s: PackedFloat32Array) -> float:
	var m := 0.0
	for x in s:
		m = max(m, abs(x))
	return m


# Fundamental frequency by autocorrelation over 60-400 Hz.
func f0(s: PackedFloat32Array) -> float:
	var rate := AudioServer.get_mix_rate()
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
