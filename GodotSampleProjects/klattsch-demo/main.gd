# The klattsch sample project.
#
# main.tscn holds the klattsch nodes you would set up in the editor:
#   Voice        a KlattschPlayer with its own KlattschStream (select it to
#                see the Voice settings, and the Audition panel to try lines
#                without running the game). With no target it plays through
#                an AudioStreamPlayer it makes itself.
#   Voice2D      a KlattschPlayer whose target is Speaker2D, an
#                AudioStreamPlayer2D, so it is panned and attenuated.
#   SamplePlayer a plain AudioStreamPlayer for pre-rendered KlattschSamples.
#
# Each tab (tabs/*.gd) shows one part of the addon. This script builds the
# window and the side panel, which shows KlattschPlayer's signals as they
# arrive: the phoneme being heard (driving a lip-sync mouth and a highlight
# in the phoneme string) and a log of every signal.
extends Control

const UI := preload("res://ui.gd")
const Mouth := preload("res://mouth.gd")
const TABS := [
	["Speak", preload("res://tabs/speak.gd")],
	["Voice", preload("res://tabs/voice.gd")],
	["Live control", preload("res://tabs/live.gd")],
	["Words & voices", preload("res://tabs/words.gd")],
	["Pre-render", preload("res://tabs/prerender.gd")],
	["Positional", preload("res://tabs/positional.gd")],
]

@onready var voice: KlattschPlayer = $Voice
@onready var voice_2d: KlattschPlayer = $Voice2D
@onready var speaker_2d: AudioStreamPlayer2D = $Speaker2D
@onready var sample_player: AudioStreamPlayer = $SamplePlayer

# The phoneme string of each utterance still speaking, by id, so that
# phrase_started's offsets can be highlighted in it.
var _phonemes := {}
var _mouth: Control
var _heard: Label
var _line: RichTextLabel
var _log: RichTextLabel
var _log_phrases: CheckBox
var _dictionary: Label


func _ready() -> void:
	_build_ui()

	# The CMU Pronouncing Dictionary (130,000 English words) is parsed on a
	# background thread. English works before it is ready, guessing from
	# spelling, so games can start speaking at once.
	Klattsch.dictionary_loaded.connect(_on_dictionary_loaded)
	Klattsch.load_dictionary()

	# The same four signals on both players. bind() adds which player sent it.
	for player: KlattschPlayer in [voice, voice_2d]:
		player.utterance_started.connect(_on_utterance_started.bind(player))
		player.phrase_started.connect(_on_phrase_started.bind(player))
		player.utterance_finished.connect(_on_utterance_finished.bind(player))
		player.error.connect(_on_error.bind(player))


# Tabs call this after starting an utterance, with the phoneme string it
# speaks, so the side panel can show and highlight it.
func track(id: int, phonemes: String) -> void:
	if id < 0:
		return
	_phonemes[id] = phonemes
	_show_line(phonemes, -1, -1)


func log_line(text: String) -> void:
	_log.append_text(text + "\n")


# The Voice settings of a KlattschStream as Klattsch.compile_with options,
# to compile or render a line in the same voice the player speaks with.
static func voice_options(stream: KlattschStream) -> Dictionary:
	return {
		"bank": stream.bank, "base_f0": stream.base_f0, "rate": stream.rate, "scale": stream.scale,
		"vibrato_depth": stream.vibrato_depth, "vibrato_rate": stream.vibrato_rate,
		"tremolo_depth": stream.tremolo_depth, "tremolo_rate": stream.tremolo_rate,
		"aspiration": stream.aspiration, "tilt": stream.tilt, "effort": stream.effort,
	}


# --- KlattschPlayer signals ---

func _on_utterance_started(id: int, player: KlattschPlayer) -> void:
	log_line("[color=lightgreen]utterance_started[/color]  id %d  (%s)" % [id, player.name])


# Emitted as each phoneme starts being heard. src_start and src_end are
# character offsets into the phoneme string the utterance was made from.
func _on_phrase_started(id: int, index: int, phoneme: String, src_start: int, src_end: int,
		player: KlattschPlayer) -> void:
	_mouth.show_phoneme(phoneme)
	_heard.text = phoneme
	if _phonemes.has(id):
		_show_line(_phonemes[id], src_start, src_end)
	if _log_phrases.button_pressed:
		log_line("phrase_started  id %d  #%d  %s  [%d, %d)  (%s)" % [id, index, phoneme, src_start, src_end, player.name])


# Emitted exactly once per id: when it has been heard to the end, cut off by
# another line, or stopped.
func _on_utterance_finished(id: int, player: KlattschPlayer) -> void:
	_phonemes.erase(id)
	if not voice.is_speaking() and not voice_2d.is_speaking():
		_mouth.close()
		_heard.text = ""
	log_line("[color=lightskyblue]utterance_finished[/color]  id %d  (%s)" % [id, player.name])


func _on_error(message: String, player: KlattschPlayer) -> void:
	log_line("[color=salmon]error[/color]  %s  (%s)" % [message, player.name])


func _on_dictionary_loaded() -> void:
	_dictionary.text = "CMU dictionary: loaded"
	log_line("[color=khaki]Klattsch.dictionary_loaded[/color]")


# --- side panel ---

# The phoneme string with [src_start, src_end) highlighted.
func _show_line(phonemes: String, from: int, to: int) -> void:
	var escape := func(s: String) -> String: return s.replace("[", "[lb]")
	if from < 0:
		_line.text = escape.call(phonemes)
		return
	_line.text = "%s[bgcolor=#4a6fa5][color=white]%s[/color][/bgcolor]%s" % [
		escape.call(phonemes.substr(0, from)), escape.call(phonemes.substr(from, to - from)),
		escape.call(phonemes.substr(to))]


func _build_ui() -> void:
	var margin := MarginContainer.new()
	margin.set_anchors_preset(Control.PRESET_FULL_RECT)
	for side in ["left", "top", "right", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, 12)
	add_child(margin)
	var columns := HBoxContainer.new()
	columns.add_theme_constant_override("separation", 12)
	margin.add_child(columns)

	var tabs := TabContainer.new()
	tabs.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	columns.add_child(tabs)
	for entry in TABS:
		var scroll := ScrollContainer.new()
		scroll.name = entry[0]
		scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
		tabs.add_child(scroll)
		var tab: VBoxContainer = entry[1].new()
		tab.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		tab.add_theme_constant_override("separation", 8)
		scroll.add_child(tab)
		tab.setup(self)

	var side := VBoxContainer.new()
	side.custom_minimum_size.x = 360
	columns.add_child(side)
	UI.heading(side, "Heard now")
	_mouth = Mouth.new()
	_mouth.custom_minimum_size = Vector2(0, 170)
	side.add_child(_mouth)
	_heard = Label.new()
	_heard.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_heard.add_theme_font_size_override("font_size", 24)
	side.add_child(_heard)
	_line = RichTextLabel.new()
	_line.bbcode_enabled = true
	_line.fit_content = true
	_line.custom_minimum_size.y = 48
	side.add_child(_line)
	UI.heading(side, "Signals")
	_log_phrases = CheckBox.new()
	_log_phrases.text = "Log phrase_started too"
	side.add_child(_log_phrases)
	_log = RichTextLabel.new()
	_log.bbcode_enabled = true
	_log.scroll_following = true
	_log.size_flags_vertical = Control.SIZE_EXPAND_FILL
	side.add_child(_log)
	_dictionary = Label.new()
	_dictionary.text = "CMU dictionary: loading..."
	_dictionary.modulate = Color(1, 1, 1, 0.7)
	side.add_child(_dictionary)
