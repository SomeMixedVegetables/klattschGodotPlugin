# Words & voices: teaching klattsch your game's words, and loading custom
# voices.
#   Klattsch.add_word / remove_word   one word at a time, at run time
#   Klattsch.load_word_list           a CMU-format file of words
#   {PH ON EM ES} in English text     phones for one occurrence only
#   Klattsch.load_bank                a voice bank (JSON) from a file
#   Klattsch.register_bank_json       a voice bank from a JSON string
extends VBoxContainer

const UI := preload("res://ui.gd")
const WORD_LIST := "res://words/game-words.txt"
const WHISPER := "res://voices/whisper.voice"

var main: Node
var _word: LineEdit
var _phones: LineEdit
var _list_line: LineEdit
var _bank_json: TextEdit
var _status: Label


func setup(main_node: Node) -> void:
	main = main_node
	_build_ui()


func _say(text: String) -> void:
	var id: int = main.voice.say(text)
	main.track(id, Klattsch.english_to_phonemes_with(text, {"intonation": main.voice.intonation}))


func _show(text: String) -> void:
	_status.text = text


# Phones are CMU style: ARPAbet codes with stress digits on vowels
# (1 primary, 2 secondary, 0 none).
func _add_word() -> void:
	if Klattsch.add_word(_word.text, _phones.text):
		_show("Added %s as %s. It wins over word lists and the dictionary." % [_word.text, _phones.text])
		_say(_word.text)
	else:
		_show("Could not add that word: see the Output panel for why.")


func _remove_word() -> void:
	var removed := Klattsch.remove_word(_word.text)
	_show(("Removed %s." if removed else "%s was not added.") % _word.text)
	_say(_word.text)


func _load_word_list() -> void:
	if Klattsch.load_word_list(WORD_LIST):
		_show("Loaded %s. Lists loaded earlier win over later ones." % WORD_LIST)
		_say(_list_line.text)


func _load_whisper() -> void:
	# Returns the bank's name, which a KlattschStream's bank property and
	# [bank=...] in phoneme strings refer to.
	var bank_name := Klattsch.load_bank(WHISPER)
	if bank_name.is_empty():
		_show("Could not load %s." % WHISPER)
		return
	main.voice.stream.bank = bank_name
	_show("Loaded the bank '%s' and set it on the Voice. Set the bank back on the Voice tab." % bank_name)
	_say("Can you hear me? I am whispering to you.")


func _register_json() -> void:
	var bank_name := Klattsch.register_bank_json(_bank_json.text)
	if bank_name.is_empty():
		_show("That JSON is not a valid bank: see the Output panel.")
		return
	_show("Registered '%s'. Speak QX with it:" % bank_name)
	var phonemes := "[bank=%s] r400 QX AA QX" % bank_name
	main.track(main.voice.say_phonemes(phonemes), phonemes)


func _build_ui() -> void:
	UI.heading(self, "Your own words")
	UI.note(self, "Names and invented words are guessed from their spelling, which is often wrong. "
		+ "Teach klattsch how to say them. Try ZORBLAX before and after loading the word list.")
	var add := UI.row(self)
	_word = UI.line_edit(add, "Word", "Xyloth")
	_phones = UI.line_edit(add, "Phones", "Z AY1 L AA0 TH")
	UI.button(add, "Add", _add_word)
	UI.button(add, "Remove", _remove_word)
	UI.button(add, "Say it", func() -> void: _say(_word.text))

	var list := UI.row(self)
	_list_line = UI.line_edit(list, "", "Zorblax the NPC loves Godot and Klattsch.")
	UI.button(list, "Say it", func() -> void: _say(_list_line.text))
	UI.button(list, "Load words/game-words.txt", _load_word_list)
	UI.note(self, "For one word in one line, write its phones in braces instead: "
		+ "\"I will {R IY1 D} it\".")

	UI.spacer(self)
	UI.heading(self, "Custom voices")
	UI.note(self, "A voice bank gives the formants of every phoneme. voices/whisper.voice extends the "
		+ "English bank and makes every sound unvoiced, so it whispers. (Give bank files an "
		+ "extension Godot does not import, like .voice; .json files are converted on import.)")
	UI.button(UI.row(self), "Load voices/whisper.voice and use it", _load_whisper)
	UI.note(self, "Banks can also come from a string, for example one your game downloads or builds. "
		+ "This one adds a new phoneme, QX, to the English bank:")
	_bank_json = UI.text_edit(self, JSON.stringify({
		"schemaVersion": 1, "name": "my-voice", "extends": "klatt1980-en",
		"phonemes": {"QX": {"voicing": 1, "F1": 500, "F2": 1500, "F3": 2500,
			"BW1": 60, "BW2": 90, "BW3": 120, "A1": 1, "A2": 0.8, "A3": 0.5}}}, "  "), 150)
	UI.button(UI.row(self), "Register this bank and say QX", _register_json)

	UI.spacer(self)
	_status = UI.note(self, "")
