# Pass or fail for the test scenes: each check that does not hold is listed
# by name in the result's "failed", and "ok" is true when none failed.
# tools/godot_tests.py (and CI) read "ok".
extends RefCounted

var failed: PackedStringArray = []


# How far timing may stray, in ms. With real audio output the signals are
# checked to a few ms of the sound; headless runs use Godot's dummy audio
# driver, which mixes in bursts of up to ~100 ms (its output latency is 0).
static func slack_ms() -> float:
	return 30.0 if AudioServer.get_output_latency() > 0.0 else 250.0


func expect(name: String, ok: bool) -> void:
	if not ok:
		failed.append(name)


func near(name: String, value: float, target: float, tolerance: float) -> void:
	expect(name, abs(value - target) <= tolerance)


func finish(r: Dictionary) -> void:
	r["failed"] = failed
	r["ok"] = failed.is_empty()
