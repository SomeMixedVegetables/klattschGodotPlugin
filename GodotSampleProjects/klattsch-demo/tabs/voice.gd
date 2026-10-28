# Voice: the KlattschStream "Voice" properties (also in the Inspector when
# you select the Voice node), presets made from them, the voice bank, and
# KlattschPlayer.intonation. Changes apply to the next line spoken.
extends VBoxContainer

const UI := preload("res://ui.gd")
# Property, label, min, max, step.
const SETTINGS := [
	["base_f0", "Base pitch (Hz)", 50.0, 400.0, 1.0],
	["rate", "Phoneme length (ms; lower is faster)", 40.0, 250.0, 1.0],
	["scale", "Formant scale (higher = smaller)", 0.6, 1.6, 0.01],
	["vibrato_depth", "Vibrato depth (Hz)", 0.0, 20.0, 0.1],
	["vibrato_rate", "Vibrato rate (Hz)", 0.5, 12.0, 0.1],
	["tremolo_depth", "Tremolo depth", 0.0, 1.0, 0.01],
	["tremolo_rate", "Tremolo rate (Hz)", 0.5, 12.0, 0.1],
	["aspiration", "Breathiness", 0.0, 1.0, 0.01],
	["tilt", "Brightness (spectral tilt)", -0.95, 0.95, 0.01],
	["effort", "Vocal effort (lax to tense)", 0.0, 1.0, 0.01],
]
# Only the properties a preset changes; the rest go back to the defaults.
const PRESETS := {
	"Default": {},
	"Robot": {"base_f0": 100.0, "rate": 95.0, "tilt": 0.6, "effort": 1.0},
	"Child": {"base_f0": 270.0, "scale": 1.25, "rate": 95.0},
	"Giant": {"base_f0": 65.0, "scale": 0.75, "rate": 170.0, "effort": 0.8},
	"Elder": {"base_f0": 110.0, "rate": 150.0, "vibrato_depth": 4.0, "vibrato_rate": 6.0,
		"tremolo_depth": 0.25, "tremolo_rate": 5.0, "aspiration": 0.35},
	"Ghost": {"base_f0": 180.0, "rate": 190.0, "vibrato_depth": 12.0, "vibrato_rate": 3.0,
		"aspiration": 0.8, "tilt": -0.5},
}
const DEFAULTS := {"base_f0": 120.0, "rate": 110.0, "scale": 1.0, "vibrato_depth": 0.0,
	"vibrato_rate": 5.0, "tremolo_depth": 0.0, "tremolo_rate": 5.0, "aspiration": 0.0,
	"tilt": 0.0, "effort": 0.5}

var main: Node
var _sliders := {}
var _bank: OptionButton
var _sample: LineEdit


func setup(main_node: Node) -> void:
	main = main_node
	_build_ui()
	_refresh_banks()
	visibility_changed.connect(_refresh_banks)


func _stream() -> KlattschStream:
	return main.voice.stream


func _apply_preset(preset_name: String) -> void:
	var values: Dictionary = DEFAULTS.merged(PRESETS[preset_name], true)
	for property in values:
		# Setting the slider sets the stream property (see _build_ui).
		_sliders[property].value = values[property]
	_say()


func _say() -> void:
	var text := _sample.text
	main.track(main.voice.say(text), Klattsch.english_to_phonemes_with(text, {"intonation": main.voice.intonation}))


# Banks registered so far: the built-in ones plus any loaded with
# Klattsch.load_bank (see the Words & voices tab).
func _refresh_banks() -> void:
	_bank.clear()
	_bank.add_item("(default: klatt1980-en)")
	for bank_name in Klattsch.get_bank_names():
		_bank.add_item(bank_name)
		if bank_name == _stream().bank:
			_bank.select(_bank.item_count - 1)


func _on_bank(index: int) -> void:
	_stream().bank = "" if index == 0 else _bank.get_item_text(index)


func _build_ui() -> void:
	UI.heading(self, "The voice")
	UI.note(self, "These are the Voice properties of the KlattschStream on the Voice node (select it in "
		+ "the editor to see them in the Inspector). They apply to the next line; for changes while "
		+ "speaking, see Live control.")
	var presets := UI.row(self)
	UI.label(presets, "Presets:")
	for preset_name in PRESETS:
		UI.button(presets, preset_name, _apply_preset.bind(preset_name))
	var try := UI.row(self)
	_sample = UI.line_edit(try, "A line to try", "The quick brown fox jumps over the lazy dog.")
	UI.button(try, "Try this voice", _say)

	UI.spacer(self)
	for s in SETTINGS:
		var property: String = s[0]
		_sliders[property] = UI.slider(self, s[1], s[2], s[3], s[4], _stream().get(property),
			func(v: float) -> void: _stream().set(property, v))

	UI.spacer(self)
	var bank_row := UI.row(self)
	UI.label(bank_row, "Voice bank:")
	_bank = UI.options(bank_row, [], _on_bank)
	UI.note(self, "A bank defines the sounds of each phoneme. ja-* banks are Japanese; load your own "
		+ "from JSON on the Words & voices tab.")
	UI.check(self, "Intonation (sentence melody for English)", main.voice.intonation,
		func(on: bool) -> void: main.voice.intonation = on)

