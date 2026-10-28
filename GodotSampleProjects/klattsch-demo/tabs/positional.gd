# Positional: a KlattschPlayer speaks through whatever player its `target`
# points at. Voice2D's target is Speaker2D, an AudioStreamPlayer2D, so its
# speech is panned and attenuated by position like any 2D sound (an
# AudioStreamPlayer3D works the same way in 3D). Its KlattschStream has its
# own voice (base_f0 150 in main.tscn).
extends VBoxContainer

const UI := preload("res://ui.gd")
const RANGE := 600.0  # pixels left and right of the listener

var main: Node
var _offset: HSlider
var _field: Control
var _sweep: Tween


func setup(main_node: Node) -> void:
	main = main_node
	_build_ui()
	_move(0.0)


# With no Camera2D, the 2D listener is the middle of the screen.
func _move(offset: float) -> void:
	var center := get_viewport().get_visible_rect().size / 2.0
	main.speaker_2d.position = center + Vector2(offset, 0)
	_field.queue_redraw()


func _say() -> void:
	var text := "I am over here. Can you tell where I am speaking from?"
	main.track(main.voice_2d.say(text), Klattsch.english_to_phonemes(text))


# Speak while gliding from one side to the other.
func _say_moving() -> void:
	if _sweep:
		_sweep.kill()
	_offset.value = -RANGE
	_say()
	_sweep = create_tween()
	_sweep.tween_property(_offset, "value", RANGE, 3.5)


func _draw_field() -> void:
	var area := _field.size
	var mid := area / 2.0
	_field.draw_rect(Rect2(Vector2.ZERO, area), Color(0.12, 0.13, 0.16))
	_field.draw_line(Vector2(0, mid.y), Vector2(area.x, mid.y), Color(1, 1, 1, 0.15), 2)
	# The listener, and the speaker at its offset scaled to the field.
	_field.draw_circle(mid, 10, Color(0.6, 0.8, 1.0))
	var x := mid.x + _offset.value / RANGE * (mid.x - 20)
	_field.draw_circle(Vector2(x, mid.y), 14, Color(1.0, 0.7, 0.3))


func _build_ui() -> void:
	UI.heading(self, "Speech in the game world")
	UI.note(self, "Voice2D is a KlattschPlayer whose target is an AudioStreamPlayer2D. Move the speaker "
		+ "(orange) around the listener (blue) and listen with headphones: the voice pans and gets "
		+ "quieter with distance, using the AudioStreamPlayer2D's own attenuation and bus settings.")
	_field = Control.new()
	_field.custom_minimum_size.y = 120
	_field.draw.connect(_draw_field)
	add_child(_field)
	_offset = UI.slider(self, "Speaker position (px from the listener)", -RANGE, RANGE, 1.0, 0.0, _move)
	var buttons := UI.row(self)
	UI.button(buttons, "Say from here", _say)
	UI.button(buttons, "Say while moving left to right", _say_moving)
	UI.button(buttons, "Stop", func() -> void: main.voice_2d.stop())
