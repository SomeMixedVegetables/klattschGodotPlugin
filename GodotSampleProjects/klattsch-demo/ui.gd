# Small helpers that build the demo's controls in code, so each tab's script
# can stay about klattsch. Nothing here is klattsch-specific.
extends RefCounted


static func heading(parent: Node, text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.add_theme_font_size_override("font_size", 18)
	parent.add_child(label)
	return label


# A wrapped paragraph explaining what the controls below it do.
static func note(parent: Node, text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	label.modulate = Color(1, 1, 1, 0.7)
	parent.add_child(label)
	return label


static func row(parent: Node) -> HBoxContainer:
	var box := HBoxContainer.new()
	box.add_theme_constant_override("separation", 8)
	parent.add_child(box)
	return box


static func button(parent: Node, text: String, pressed: Callable) -> Button:
	var b := Button.new()
	b.text = text
	b.pressed.connect(pressed)
	parent.add_child(b)
	return b


static func check(parent: Node, text: String, on: bool, toggled: Callable) -> CheckButton:
	var c := CheckButton.new()
	c.text = text
	c.button_pressed = on
	c.toggled.connect(toggled)
	parent.add_child(c)
	return c


static func options(parent: Node, items: Array, selected: Callable) -> OptionButton:
	var o := OptionButton.new()
	for item in items:
		o.add_item(str(item))
	o.item_selected.connect(selected)
	parent.add_child(o)
	return o


static func line_edit(parent: Node, placeholder: String, text := "") -> LineEdit:
	var e := LineEdit.new()
	e.placeholder_text = placeholder
	e.text = text
	e.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	parent.add_child(e)
	return e


static func text_edit(parent: Node, text: String, height := 80.0) -> TextEdit:
	var t := TextEdit.new()
	t.text = text
	t.custom_minimum_size.y = height
	t.wrap_mode = TextEdit.LINE_WRAPPING_BOUNDARY
	parent.add_child(t)
	return t


# A labelled slider with its value shown; `changed` gets the new value.
static func slider(parent: Node, label: String, min_value: float, max_value: float, step: float,
		value: float, changed: Callable) -> HSlider:
	var box := row(parent)
	var name_label := Label.new()
	name_label.text = label
	name_label.custom_minimum_size.x = 300
	box.add_child(name_label)
	var s := HSlider.new()
	s.min_value = min_value
	s.max_value = max_value
	s.step = step
	s.value = value
	s.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	s.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	box.add_child(s)
	var value_label := Label.new()
	value_label.custom_minimum_size.x = 56
	value_label.text = str(value)
	box.add_child(value_label)
	s.value_changed.connect(func(v: float) -> void:
		value_label.text = str(snappedf(v, step))
		changed.call(v))
	return s


static func label(parent: Node, text: String) -> Label:
	var l := Label.new()
	l.text = text
	parent.add_child(l)
	return l


static func spacer(parent: Node, height := 8.0) -> void:
	var c := Control.new()
	c.custom_minimum_size.y = height
	parent.add_child(c)
