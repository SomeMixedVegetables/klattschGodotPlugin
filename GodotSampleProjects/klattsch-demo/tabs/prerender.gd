# Pre-render: compile a line once, render it to a KlattschSample (an
# AudioStream you can play on any player, as often as you like), and save
# WAV files.
#   Klattsch.compile / compile_with   phonemes -> KlattschUtterance
#   Klattsch.render                   utterance -> KlattschSample
#   Klattsch.save_wav                 utterance -> .wav file
#   KlattschSample.render_mode        GAME follows pitch_scale; EXACT plays
#                                     the samples untouched
#   KlattschSample.to_audio_stream_wav   a stock AudioStreamWAV copy
extends VBoxContainer

const UI := preload("res://ui.gd")
const WAV_PATH := "user://klattsch-line.wav"

var main: Node
var _text: TextEdit
var _info: Label
var _phrases: ItemList
var _pitch: HSlider
var _exact: CheckButton
var _from: HSlider
var _utterance: KlattschUtterance
var _sample: KlattschSample


func setup(main_node: Node) -> void:
	main = main_node
	_build_ui()


func render() -> void:
	# The phoneme string, compiled with the Voice node's settings so it
	# sounds like the live voice (main.voice_options builds the options).
	var phonemes := Klattsch.english_to_phonemes(_text.text)
	_utterance = Klattsch.compile_with(phonemes, main.voice_options(main.voice.stream))
	if _utterance == null:
		return
	_sample = Klattsch.render(_utterance)  # at the mix rate
	_sample.render_mode = KlattschSample.EXACT if _exact.button_pressed else KlattschSample.GAME
	_from.max_value = _sample.get_length()
	_info.text = "%.2f s, %d phrases, %d samples at %d Hz%s" % [
		_sample.get_length(), _utterance.get_phrases().size(), _sample.get_samples().size(),
		_sample.get_sample_rate(),
		"" if _utterance.get_warnings().is_empty() else "\nWarnings: " + ", ".join(_utterance.get_warnings())]
	# Phrase timing, for subtitles or lip-sync without a KlattschPlayer.
	_phrases.clear()
	for p: Dictionary in _utterance.get_phrases():
		_phrases.add_item("%7.0f ms  %s" % [p.t_start_ms, p.phoneme])


func play() -> void:
	if _sample == null:
		render()
	var player: AudioStreamPlayer = main.sample_player
	player.stream = _sample
	player.pitch_scale = _pitch.value
	player.play(_from.value)


func play_wav() -> void:
	if _sample == null:
		render()
	var player: AudioStreamPlayer = main.sample_player
	player.stream = _sample.to_audio_stream_wav()
	player.pitch_scale = _pitch.value
	player.play(_from.value)


func save() -> void:
	if _utterance == null:
		render()
	# 16-bit, peak-normalized, with the text as the file's comment.
	var err := Klattsch.save_wav(_utterance, WAV_PATH, _text.text)
	_info.text = ("Saved %s" % ProjectSettings.globalize_path(WAV_PATH)) if err == OK else "Error %d" % err


func _build_ui() -> void:
	UI.heading(self, "Render once, play many times")
	UI.note(self, "For lines that never change (menus, barks, a radio message), render them ahead of "
		+ "time: no synthesis while playing, and a KlattschSample works with any AudioStreamPlayer, "
		+ "2D or 3D, like an imported sound.")
	_text = UI.text_edit(self, "Mission accomplished. Return to base for your next assignment.", 60)
	var actions := UI.row(self)
	UI.button(actions, "Render", render)
	UI.button(actions, "Play", play)
	UI.button(actions, "Play as AudioStreamWAV", play_wav)
	UI.button(actions, "Stop", func() -> void: main.sample_player.stop())
	_info = UI.note(self, "Render to see the length and phrase timing.")
	_pitch = UI.slider(self, "Pitch scale (game mode only)", 0.5, 2.0, 0.01, 1.0,
		func(v: float) -> void: main.sample_player.pitch_scale = v)
	_from = UI.slider(self, "Play from (s)", 0.0, 1.0, 0.01, 0.0, func(_v: float) -> void: pass)
	_exact = UI.check(self, "Exact mode (bit-exact samples; ignores pitch scale)", false,
		func(on: bool) -> void:
			if _sample:
				_sample.render_mode = KlattschSample.EXACT if on else KlattschSample.GAME)

	UI.spacer(self)
	UI.heading(self, "WAV files")
	var wav := UI.row(self)
	UI.button(wav, "Save %s" % WAV_PATH, save)
	UI.button(wav, "Open the folder", func() -> void: OS.shell_open(ProjectSettings.globalize_path("user://")))

	UI.spacer(self)
	UI.heading(self, "Phrase timing")
	_phrases = ItemList.new()
	_phrases.custom_minimum_size.y = 180
	add_child(_phrases)
