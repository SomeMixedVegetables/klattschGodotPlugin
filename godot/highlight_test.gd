# Step 8 checks, run time (docs/klattsch-godot-build-spec.docx):
# Klattsch.highlight and KlattschSyntaxHighlighter on a TextEdit, including
# lines recoloured by an edit on another line. Prints one STEP8_RESULT JSON
# line and quits. The editor half is addons/klattsch_editor_test.
extends Node

var checks := {}


func check(name: String, ok: bool, detail = null) -> void:
	checks[name] = ok if detail == null else [ok, detail]


func color_at(h: SyntaxHighlighter, line: int, column: int) -> Color:
	var runs: Dictionary = h.get_line_syntax_highlighting(line)
	var best := -1
	for c in runs:
		if c <= column and c > best:
			best = c
	return runs[best]["color"] if best >= 0 else Color.TRANSPARENT


func _ready() -> void:
	# 1. Klattsch.highlight: spans cover the text, classes as klattsch's own.
	var spans: Array = Klattsch.highlight("HH AH' b220 ZZ # hi")
	var classes := spans.map(func(s): return s["class"])
	check("highlight_classes", classes == ["hl-phoneme", "", "hl-phoneme", "hl-stress", "",
		"hl-directive hl-d-pitch", "hl-value hl-v-pitch", "", "hl-badphoneme", "", "hl-comment"], classes)
	check("highlight_covers", spans[-1]["end"] == 19 and spans[0]["start"] == 0)
	var ja: Array = Klattsch.highlight("A I U", "ja-mokhtari-2000")
	check("highlight_bank", ja.all(func(s): return s["class"] != "hl-badphoneme"), ja)

	# 2. A TextEdit with KlattschSyntaxHighlighter.
	var edit := TextEdit.new()
	edit.size = Vector2(600, 200)
	add_child(edit)
	var h := KlattschSyntaxHighlighter.new()
	edit.syntax_highlighter = h
	edit.text = "HH AH'\nAA b220\nZZ"
	await get_tree().process_frame
	check("phoneme_color", color_at(h, 0, 0) == h.phoneme_color)
	check("stress_color", color_at(h, 0, 5) == h.stress_color)
	check("pitch_color", color_at(h, 1, 4) == h.pitch_color)
	check("error_color", color_at(h, 2, 0) == h.error_color)
	check("plain_color", color_at(h, 0, 2) == edit.get_theme_color("font_color"))

	# Opening a block comment on line 0 recolours lines 1 and 2, which the
	# TextEdit's own cache still holds.
	edit.set_caret_line(0)
	edit.set_caret_column(0)
	edit.insert_text_at_caret("/* ")
	edit.get_line(0)
	await get_tree().process_frame
	await get_tree().process_frame
	check("block_comment_reaches_later_lines",
		color_at(h, 2, 0) == h.comment_color and color_at(h, 1, 0) == h.comment_color)

	# A bank switch on one line changes which phonemes are errors below it.
	edit.text = "A I\n[bank=ja-mokhtari-2000]\nA I"
	await get_tree().process_frame
	check("bank_switch", color_at(h, 0, 0) == h.error_color and color_at(h, 2, 0) == h.phoneme_color)

	# The bank property, and colour properties, repaint.
	h.bank = "ja-mokhtari-2000"
	h.phoneme_color = Color.YELLOW
	await get_tree().process_frame
	check("bank_property", color_at(h, 0, 0) == Color.YELLOW)

	var ok := checks.values().all(func(v): return v is bool and v or v is Array and v[0])
	print("STEP8_RESULT ", JSON.stringify({"ok": ok, "checks": checks}))
	get_tree().quit()
