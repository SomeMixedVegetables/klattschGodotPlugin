# Step 8 checks, editor half (docs/klattsch-godot-build-spec.docx): the
# script editor's "klattsch phonemes" highlighter and the Inspector's
# audition panel, driven through the real editor UI. Does nothing unless the
# editor is started with `-- --klattsch-editor-test`; then prints one
# STEP8_EDITOR_RESULT JSON line and quits. With `--screenshot=<png>` it also
# saves the editor window (run without --headless for that).
@tool
extends EditorPlugin

var checks := {}


func _enter_tree() -> void:
	if "--klattsch-editor-test" in OS.get_cmdline_user_args():
		_run.call_deferred()


func check(name: String, ok: bool, detail = null) -> void:
	checks[name] = ok if detail == null else [ok, detail]


func frames(n: int) -> void:
	for i in n:
		await get_tree().process_frame


func find_all(node: Node, test: Callable, out: Array = []) -> Array:
	if test.call(node):
		out.append(node)
	for child in node.get_children(true):
		find_all(child, test, out)
	return out


func panels() -> Array:
	return find_all(EditorInterface.get_inspector(), func(n): return n.get_class() == "KlattschAuditionPanel")


func child_of(panel: Node, cls: String, text := "") -> Node:
	var found := find_all(panel, func(n): return n.is_class(cls) and (text == "" or n.text == text))
	return found[0] if found else null


func status(panel: Node) -> String:
	return find_all(panel, func(n): return n is Label and n.text != "Audition")[0].text


func set_mode(panel: Node, index: int) -> void:
	var mode: OptionButton = child_of(panel, "OptionButton")
	mode.select(index)
	mode.item_selected.emit(index)


# Press Play and wait for the status line; returns the stream now playing
# (a 16-bit AudioStreamWAV copy of the rendered line).
func play(panel: Node) -> AudioStreamWAV:
	child_of(panel, "Button", "Play").pressed.emit()
	for i in 600:
		if " s, " in status(panel) or "Not " in status(panel):
			break
		await get_tree().process_frame
	var players := find_all(get_tree().root,
		func(n): return n is AudioStreamPlayer and n.playing and n.stream is AudioStreamWAV)
	return players[0].stream if players.size() == 1 else null


# The panel's sound is exactly this render, quantized as to_audio_stream_wav does.
func same(a: AudioStreamWAV, b: KlattschSample) -> bool:
	return a != null and b != null and a.data == b.to_audio_stream_wav().data


func _run() -> void:
	await frames(5)

	# 1. The script editor offers the highlighter (Edit > Syntax Highlighter).
	EditorInterface.edit_script(load("res://demo.gd"))
	await frames(5)
	# The Syntax Highlighter submenu fills itself as it opens.
	var names := []
	for menu in find_all(EditorInterface.get_script_editor(), func(n): return n is PopupMenu):
		for i in menu.item_count:
			if menu.get_item_text(i) == "Syntax Highlighter" and menu.get_item_submenu_node(i):
				var sub: PopupMenu = menu.get_item_submenu_node(i)
				sub.about_to_popup.emit()
				for j in sub.item_count:
					names.append(sub.get_item_text(j))
	check("script_editor_menu", "klattsch phonemes" in names, names)
	var code := CodeEdit.new()
	add_child(code)
	code.syntax_highlighter = KlattschEditorSyntaxHighlighter.new()
	code.text = "AA b220 ZZ"
	await frames(2)
	var runs: Dictionary = code.syntax_highlighter.get_line_syntax_highlighting(0)
	check("editor_highlighter_runs", runs.size() == 5, runs)
	code.queue_free()
	if "--only-highlighter" in OS.get_cmdline_user_args():
		print("STEP8_EDITOR_RESULT ", JSON.stringify(checks))
		get_tree().quit()
		return

	# 2. The audition panel on a KlattschPlayer with its own stream.
	EditorInterface.open_scene_from_path("res://audition_test.tscn")
	await frames(5)
	var root := EditorInterface.get_edited_scene_root()
	EditorInterface.edit_node(root.get_node("Voice"))
	await frames(5)
	var found := panels()
	check("panel_once", found.size() == 1, found.size())
	var panel: Node = found[0]
	var text: CodeEdit = child_of(panel, "CodeEdit")
	var voice := {"base_f0": 180.0, "rate": 95.0, "vibrato_depth": 3.0}

	set_mode(panel, 0)
	text.text = "Hello there."
	text.text_changed.emit()
	var heard := await play(panel)
	var english := Klattsch.english_to_phonemes("Hello there.")
	check("english_voice", same(heard, Klattsch.render(Klattsch.compile_with(english, voice))), status(panel))

	child_of(panel, "Button", "To Phonemes").pressed.emit()
	await frames(2)
	check("convert_english", text.text == english and child_of(panel, "OptionButton").selected == 2, text.text)
	check("phoneme_highlighter", text.syntax_highlighter is KlattschSyntaxHighlighter)
	heard = await play(panel)
	check("phonemes_voice", same(heard, Klattsch.render(Klattsch.compile_with(english, voice))))

	set_mode(panel, 1)
	text.text = "こんにちは"
	text.text_changed.emit()
	heard = await play(panel)
	var japanese := Klattsch.japanese_to_phonemes("こんにちは")
	var ja_voice := voice.merged({"bank": "ja-mokhtari-2000"}, true)
	check("japanese_voice", same(heard, Klattsch.render(Klattsch.compile_with(japanese, ja_voice))), status(panel))
	check("no_highlighter_for_text", text.syntax_highlighter == null)
	child_of(panel, "Button", "To Phonemes").pressed.emit()
	await frames(2)
	check("convert_japanese", text.text == "[bank=ja-mokhtari-2000] " + japanese, text.text)

	set_mode(panel, 2)
	text.text = "AA QQ"
	text.text_changed.emit()
	await play(panel)
	check("warning_shown", "\n" in status(panel), status(panel))
	child_of(panel, "Button", "Stop").pressed.emit()
	await frames(2)
	check("stop", find_all(get_tree().root,
		func(n): return n is AudioStreamPlayer and n.playing and n.stream is AudioStreamWAV).is_empty())

	# Text is kept per mode across panels.
	EditorInterface.edit_node(root.get_node("Bare"))
	await frames(5)
	panel = panels()[0]
	text = child_of(panel, "CodeEdit")
	check("text_kept", text.text == "AA QQ", text.text)

	# 3. A KlattschPlayer whose voice is its target's KlattschStream, with
	# intonation off.
	set_mode(panel, 0)
	heard = await play(panel)
	var flat := Klattsch.english_to_phonemes_with("Hello there.", {"intonation": false})
	check("target_stream_voice", same(heard, Klattsch.render(Klattsch.compile_with(flat, {"base_f0": 90.0}))), status(panel))

	# 4. A KlattschStream on its own.
	EditorInterface.edit_resource(root.get_node("Voice").stream)
	await frames(5)
	found = panels()
	check("stream_panel", found.size() == 1, found.size())
	if found.size() == 1:
		heard = await play(found[0])
		check("stream_voice", same(heard, Klattsch.render(Klattsch.compile_with(english, voice))))

	# 5. Back on the player, its stream sub-resource gets no second panel.
	EditorInterface.edit_node(root.get_node("Voice"))
	await frames(5)
	for picker in find_all(EditorInterface.get_inspector(), func(n): return n is EditorResourcePicker):
		if picker.edited_resource is KlattschStream:
			picker.resource_selected.emit(picker.edited_resource, true)
	await frames(5)
	check("no_nested_panel", panels().size() == 1, panels().size())

	for arg in OS.get_cmdline_user_args():
		if arg.begins_with("--screenshot="):
			EditorInterface.edit_node(root.get_node("Voice"))
			panel = panels()[0]
			set_mode(panel, 2)
			child_of(panel, "CodeEdit").text = "/* greeting */ b140 HH AH' L OW+20 , [rate=120] W ER' L D ZZ ."
			await frames(10)
			get_viewport().get_texture().get_image().save_png(arg.trim_prefix("--screenshot="))

	var ok := checks.values().all(func(v): return v is bool and v or v is Array and v[0])
	print("STEP8_EDITOR_RESULT ", JSON.stringify({"ok": ok, "checks": checks}))
	get_tree().quit(0 if ok else 1)
