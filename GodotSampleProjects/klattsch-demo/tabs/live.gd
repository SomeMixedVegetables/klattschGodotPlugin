# Live control: steering the synthesizer directly, without compiling a line.
# KlattschPlayer.set_param() and KlattschStream.set_params() glide any of
# the 19 synth parameters (F0, voicing, F1-F3, BW1-BW3, A1-A3, gain,
# vibratoDepth, vibratoRate, tremoloDepth, tremoloRate, aspiration, tilt,
# effort) to a new value over glide_ms. silence() fades out; reset() returns
# to the silent defaults. Useful for sirens, creature noises, pitch bends on
# an alarm, or a voice that reacts to gameplay.
extends VBoxContainer

const UI := preload("res://ui.gd")
# Formants from the klatt1980-en bank: F1-F3 (Hz) and bandwidths BW1-BW3.
const VOWELS := {
	"AA (father)": [700, 1220, 2600, 130, 70, 160],
	"IY (see)": [310, 2020, 2960, 45, 200, 400],
	"UW (blue)": [350, 1250, 2200, 65, 110, 140],
	"EH (bed)": [530, 1680, 2500, 60, 90, 200],
	"OW (go)": [540, 1100, 2300, 80, 70, 70],
	"ER (her)": [470, 1270, 1540, 100, 60, 110],
}

var main: Node
var _glide: HSlider
var _pitch: HSlider


func setup(main_node: Node) -> void:
	main = main_node
	_build_ui()


func _stream() -> KlattschStream:
	return main.voice.stream


# Turn the voice on and shape it into a vowel. Voice 0 is the only voice
# unless a line uses [voice=N].
func hold_vowel(formants: Array) -> void:
	# Values must be floats: set_params rejects ints (700 rather than 700.0).
	_stream().set_params({
		"voicing": 1.0, "F0": _pitch.value,
		"F1": float(formants[0]), "F2": float(formants[1]), "F3": float(formants[2]),
		"BW1": float(formants[3]), "BW2": float(formants[4]), "BW3": float(formants[5]),
		"A1": 1.0, "A2": 0.9, "A3": 0.7,
	}, _glide.value, 0)


func _build_ui() -> void:
	UI.heading(self, "Play the synthesizer live")
	UI.note(self, "Click a vowel to start a sustained sound, then click others to glide between them and "
		+ "move the sliders. Every change glides over the glide time. Try the sliders while a line is "
		+ "speaking too (Speak tab): live changes last until the line's next phoneme.")
	var vowels := UI.row(self)
	for vowel_name in VOWELS:
		UI.button(vowels, vowel_name, hold_vowel.bind(VOWELS[vowel_name]))
	var stop := UI.row(self)
	UI.button(stop, "Silence (fade out)", func() -> void: _stream().silence())
	UI.button(stop, "Reset (silent defaults)", func() -> void: _stream().reset())

	UI.spacer(self)
	_glide = UI.slider(self, "Glide time (ms)", 0.0, 2000.0, 10.0, 150.0, func(_v: float) -> void: pass)
	# KlattschPlayer.set_param is the same as its stream's set_param.
	_pitch = UI.slider(self, "Pitch: F0 (Hz)", 50.0, 500.0, 1.0, 140.0,
		func(v: float) -> void: main.voice.set_param(&"F0", v, _glide.value))
	UI.slider(self, "Vibrato depth (Hz)", 0.0, 30.0, 0.5, 0.0,
		func(v: float) -> void: main.voice.set_param(&"vibratoDepth", v, _glide.value))
	UI.slider(self, "Vibrato rate (Hz)", 0.5, 15.0, 0.1, 5.0,
		func(v: float) -> void: main.voice.set_param(&"vibratoRate", v, _glide.value))
	UI.slider(self, "Tremolo depth", 0.0, 1.0, 0.01, 0.0,
		func(v: float) -> void: main.voice.set_param(&"tremoloDepth", v, _glide.value))
	UI.slider(self, "Breathiness: aspiration", 0.0, 1.0, 0.01, 0.0,
		func(v: float) -> void: main.voice.set_param(&"aspiration", v, _glide.value))
	UI.slider(self, "Brightness: tilt", -0.95, 0.95, 0.01, 0.0,
		func(v: float) -> void: main.voice.set_param(&"tilt", v, _glide.value))
	UI.slider(self, "Effort", 0.0, 1.0, 0.01, 0.5,
		func(v: float) -> void: main.voice.set_param(&"effort", v, _glide.value))
	UI.slider(self, "Loudness: gain", 0.0, 2.0, 0.01, 1.0,
		func(v: float) -> void: main.voice.set_param(&"gain", v, _glide.value))

	UI.spacer(self)
	UI.heading(self, "A siren, in code")
	UI.note(self, "A held vowel and two set_param calls a second apart make a siren: each glide takes "
		+ "the whole second.")
	UI.button(UI.row(self), "Siren (4 seconds)", _siren)


func _siren() -> void:
	hold_vowel(VOWELS["UW (blue)"])
	for i in 2:
		main.voice.set_param(&"F0", 600.0, 1000.0)
		await get_tree().create_timer(1.0).timeout
		main.voice.set_param(&"F0", 300.0, 1000.0)
		await get_tree().create_timer(1.0).timeout
	_stream().silence()
