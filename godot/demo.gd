# KlattschPlayer demo: the phoneme being heard is highlighted as it plays,
# to check by ear that phrase_started lines up with the sound.
#   Space  say three lines (queued)
#   I      interrupt with another line
#   P      pause / resume the tree (signals and sound freeze together)
#   Up/Dn  glide the pitch live
# Test scenes can be run through it: -- --scene=res://lang_test.tscn
extends Control

const LINES := [
	"b140 HH AH L OW , W ER L D",
	"b150 DH IH S IH Z K L AE CH",
	"b130 T AO K IH NG IH N G OW D OW",
]

@onready var voice: KlattschPlayer = $KlattschPlayer
@onready var text: RichTextLabel = $Text
@onready var status: Label = $Status

var phonemes := {}  # utterance id -> phoneme string
var pitch := 140.0


func _ready() -> void:
	# Exported builds take no scene on the command line; run a test scene
	# with `-- --scene=res://lang_test.tscn` instead.
	for arg in OS.get_cmdline_user_args():
		if arg.begins_with("--scene="):
			get_tree().change_scene_to_file.call_deferred(arg.trim_prefix("--scene="))
			return
	process_mode = Node.PROCESS_MODE_ALWAYS  # keys work while paused
	voice.process_mode = Node.PROCESS_MODE_PAUSABLE
	voice.queue_mode = KlattschPlayer.QUEUE
	voice.phrase_started.connect(_on_phrase)
	voice.utterance_started.connect(func(id): status.text = "speaking %d" % id)
	voice.utterance_finished.connect(func(id): status.text = "finished %d" % id)
	voice.error.connect(func(m): status.text = m)
	text.text = "Space: speak   I: interrupt   P: pause   Up/Down: pitch"


func _unhandled_input(event: InputEvent) -> void:
	if not (event is InputEventKey and event.pressed and not event.echo):
		return
	match event.keycode:
		KEY_SPACE:
			voice.queue_mode = KlattschPlayer.QUEUE
			for line in LINES:
				say(line)
		KEY_I:
			voice.queue_mode = KlattschPlayer.INTERRUPT
			say("b170 W EY T , W AH T IH Z DH AE T")
		KEY_P:
			get_tree().paused = not get_tree().paused
			status.text = "paused" if get_tree().paused else "resumed"
		KEY_UP:
			pitch += 30.0
			voice.set_param(&"F0", pitch, 80.0)
		KEY_DOWN:
			pitch = max(60.0, pitch - 30.0)
			voice.set_param(&"F0", pitch, 80.0)


func say(line: String) -> void:
	var id := voice.say_phonemes(line)
	if id > 0:
		phonemes[id] = line


func _on_phrase(id: int, _index: int, phoneme: String, src_start: int, src_end: int) -> void:
	var line: String = phonemes.get(id, "")
	if phoneme.is_empty():
		text.text = line  # a pause
		return
	text.text = "%s[b][color=yellow]%s[/color][/b]%s" % [
		line.substr(0, src_start), line.substr(src_start, src_end - src_start), line.substr(src_end)]
