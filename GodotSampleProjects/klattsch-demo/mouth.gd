# A cartoon mouth driven by KlattschPlayer.phrase_started: each phoneme
# sets how open and how rounded the lips should be, and the shape eases
# there. This is the simplest kind of lip-sync; a real character would pick
# a mouth sprite or blend shape per phoneme the same way.
extends Control

# Phoneme code -> [openness 0..1, width 0..1]. Codes are klattsch's
# (ARPAbet for English; A I U E O for Japanese). Anything else is a
# half-open consonant.
const SHAPES := {
	"AA": [1.0, 0.9], "AE": [0.9, 1.0], "AH": [0.8, 0.85], "AO": [0.9, 0.6], "AW": [0.9, 0.7],
	"AY": [0.9, 0.9], "EH": [0.6, 1.0], "ER": [0.5, 0.6], "EY": [0.55, 1.0], "IH": [0.45, 1.0],
	"IY": [0.35, 1.0], "OW": [0.6, 0.45], "OY": [0.7, 0.5], "UH": [0.45, 0.5], "UW": [0.35, 0.35],
	"W": [0.25, 0.3], "M": [0.0, 0.8], "B": [0.0, 0.8], "P": [0.0, 0.8], "F": [0.1, 0.9],
	"V": [0.1, 0.9], "A": [1.0, 0.9], "I": [0.35, 1.0], "U": [0.35, 0.4], "E": [0.6, 1.0],
	"O": [0.6, 0.45],
}

var _open := 0.0
var _width := 0.8
var _target_open := 0.0
var _target_width := 0.8


func show_phoneme(code: String) -> void:
	var shape: Array = SHAPES.get(code, [0.3, 0.8])
	_target_open = shape[0]
	_target_width = shape[1]


func close() -> void:
	_target_open = 0.0
	_target_width = 0.8


func _process(delta: float) -> void:
	var k := 1.0 - exp(-delta * 25.0)
	_open = lerpf(_open, _target_open, k)
	_width = lerpf(_width, _target_width, k)
	queue_redraw()


func _draw() -> void:
	var center := size / 2.0
	var face := minf(size.x, size.y) * 0.45
	draw_circle(center, face, Color(0.98, 0.82, 0.62))
	draw_circle(center + Vector2(-face * 0.38, -face * 0.3), face * 0.09, Color(0.15, 0.15, 0.2))
	draw_circle(center + Vector2(face * 0.38, -face * 0.3), face * 0.09, Color(0.15, 0.15, 0.2))
	var mouth := center + Vector2(0, face * 0.38)
	var w := face * (0.25 + 0.35 * _width)
	var h := maxf(face * 0.45 * _open, 2.0)
	var points := PackedVector2Array()
	for i in 32:
		var a := TAU * i / 32.0
		points.append(mouth + Vector2(cos(a) * w, sin(a) * h))
	draw_colored_polygon(points, Color(0.45, 0.1, 0.12))
