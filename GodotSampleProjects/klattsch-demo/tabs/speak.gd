# Speak: KlattschPlayer.say (English), say_japanese (kana or romaji) and
# say_phonemes (klattsch phoneme strings), interrupting or queueing, and
# stop(). Also shows the phoneme string each line becomes, and how each
# English word was pronounced (Klattsch.english_words).
extends VBoxContainer

const UI := preload("res://ui.gd")
enum { ENGLISH, JAPANESE, PHONEMES }
const EXAMPLES := [
	["English", ENGLISH, "Hello there! Welcome to klattsch, a tiny speech synthesizer for Godot."],
	["Numbers, money and times", ENGLISH, "Dr. Smith paid $4.50 at 3:30 p.m. on March 21st, 1999."],
	["A question", ENGLISH, "Are you sure you want to open the door?"],
	["Inline phones", ENGLISH, "I will {R IY1 D} the book that I {R EH1 D} yesterday."],
	["Japanese (kana)", JAPANESE, "こんにちは、げんきですか"],
	["Japanese (romaji)", JAPANESE, "arigatou gozaimasu"],
	["Phonemes", PHONEMES, "HH AH L OW' , W ER' L D ."],
	["Phonemes with directives", PHONEMES, "b180 r80 HH IY' Z b120 r160 G OW' N . b220 W AH'+30 T ."],
	["Singing (notes)", PHONEMES, "r300 bC4 L AA bD4 L AA bE4 L AA bG4 L AA' r600 bC5 L AA"],
	["Two voices", PHONEMES, "bC3 AA r600 AA [voice=1] bG3 OW r600 OW"],
]

var main: Node
var _mode: OptionButton
var _text: TextEdit
var _phonemes: TextEdit
var _words: Tree
var _highlighter := KlattschSyntaxHighlighter.new()


func setup(main_node: Node) -> void:
	main = main_node
	_build_ui()
	_on_example(0)


func say() -> void:
	var text := _text.text.strip_edges()
	if text.is_empty():
		return
	var voice: KlattschPlayer = main.voice
	var id := -1
	var phonemes := ""
	match _mode.selected:
		ENGLISH:
			# The one-liner most games need.
			id = voice.say(text)
			# The same conversion say() does, to show what it said.
			phonemes = Klattsch.english_to_phonemes_with(text, {"intonation": voice.intonation})
			_show_words(text)
		JAPANESE:
			# Uses the stream's bank if it is a Japanese one (ja-*),
			# otherwise ja-mokhtari-2000.
			id = voice.say_japanese(text)
			phonemes = Klattsch.japanese_to_phonemes(text)
		PHONEMES:
			id = voice.say_phonemes(text)
			phonemes = text
	_phonemes.text = phonemes
	main.track(id, phonemes)


func _on_mode(mode: int) -> void:
	# Phoneme strings get syntax highlighting; unknown phonemes show in red.
	_text.syntax_highlighter = _highlighter if mode == PHONEMES else null
	_words.get_parent().visible = mode == ENGLISH


func _on_example(index: int) -> void:
	var example: Array = EXAMPLES[index]
	_mode.select(example[1])
	_on_mode(example[1])
	_text.text = example[2]


func _on_queue_toggled(on: bool) -> void:
	main.voice.queue_mode = KlattschPlayer.QUEUE if on else KlattschPlayer.INTERRUPT


# How each word was pronounced: from the dictionary, as a number, spelled
# out, sounded out from its spelling, and so on.
func _show_words(text: String) -> void:
	_words.clear()
	var root := _words.create_item()
	for word: Dictionary in Klattsch.english_words(text):
		var item := _words.create_item(root)
		item.set_text(0, word.text)
		item.set_text(1, word.source)
		item.set_text(2, word.phonemes)


func _build_ui() -> void:
	UI.heading(self, "Say a line")
	UI.note(self, "KlattschPlayer.say() speaks English, say_japanese() kana or romaji, and say_phonemes() "
		+ "klattsch phoneme strings, in the voice set on the Voice tab. Each returns an id that the "
		+ "signals (right) report. Press Say again while it speaks: by default a new line interrupts; "
		+ "with Queue on, lines wait their turn and follow without a gap.")
	var top := UI.row(self)
	var names := []
	for e in EXAMPLES:
		names.append(e[0])
	UI.label(top, "Example:")
	UI.options(top, names, _on_example)
	UI.label(top, "  Text is:")
	_mode = UI.options(top, ["English", "Japanese", "Phonemes"], _on_mode)
	_text = UI.text_edit(self, "", 90)
	_text.gui_input.connect(func(event: InputEvent) -> void:
		if event is InputEventKey and event.pressed and event.keycode == KEY_ENTER and event.is_command_or_control_pressed():
			_text.accept_event()
			say())
	var buttons := UI.row(self)
	UI.button(buttons, "Say  (Ctrl+Enter)", say)
	UI.button(buttons, "Stop", func() -> void: main.voice.stop())
	UI.check(buttons, "Queue instead of interrupting", false, _on_queue_toggled)

	UI.spacer(self)
	UI.heading(self, "What it said, as phonemes")
	UI.note(self, "Klattsch.english_to_phonemes() and japanese_to_phonemes() give the phoneme string a "
		+ "line becomes. Copy it to Phonemes mode to hand-tune stress (') pitch (b120, +20) and timing (r90).")
	_phonemes = UI.text_edit(self, "", 70)
	_phonemes.editable = false
	_phonemes.syntax_highlighter = KlattschSyntaxHighlighter.new()

	var words := VBoxContainer.new()
	add_child(words)
	UI.spacer(words)
	UI.heading(words, "How each word was pronounced")
	_words = Tree.new()
	_words.columns = 3
	_words.hide_root = true
	_words.column_titles_visible = true
	_words.set_column_title(0, "Word")
	_words.set_column_title(1, "Source")
	_words.set_column_title(2, "Phonemes")
	_words.custom_minimum_size.y = 170
	words.add_child(_words)
