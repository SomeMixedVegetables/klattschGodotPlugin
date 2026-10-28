"""Run the Godot test scenes (godot/) headless against a packaged addon.

    python tools/godot_tests.py --godot <Godot executable> --addon dist/klattsch-godot \
        [--cli target/release/klattsch] [--work <dir>]

Copies the test project to a work folder with the packaged addons/klattsch
in place of the debug build (godot/klattsch.gdextension), imports it, runs
each test scene and the editor checks (STEP8_EDITOR_RESULT), and fails
unless every one prints a result with "ok": true and exits cleanly. With
--cli, the WAV files Klattsch.save_wav wrote are also compared byte for
byte with klattsch-cli's output for the same arguments.

Headless Godot uses its dummy audio driver: audio is mixed (in bursts), so
exactness is checked as with real output; timing is checked more loosely
(see godot/verdict.gd).
"""

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCENES = ["speech", "exact", "player_test", "lang_test", "sample_test", "highlight_test"]
TIMEOUT = 600


def run(cmd, timeout=TIMEOUT, cwd=None):
    """Run a command; return (exit code, output)."""
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout,
                           cwd=cwd)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return "timeout", out


def result(output, tag):
    for line in output.splitlines():
        if line.startswith(tag + " "):
            return json.loads(line[len(tag) + 1:])
    return None


def make_project(addon, work):
    project = work / "project"
    if project.exists():
        shutil.rmtree(project)
    ignore = shutil.ignore_patterns(".godot", "klattsch.gdextension", "klattsch.gdextension.uid", "klattsch")
    shutil.copytree(ROOT / "godot", project, ignore=ignore)
    shutil.copytree(addon / "addons" / "klattsch", project / "addons" / "klattsch")
    return project


def compare_wavs(r, cli, work):
    """Each WAV save_wav wrote against klattsch-cli's; returns failures."""
    failures = []
    for name, wav in r.get("wavs", {}).items():
        out = work / f"cli-{name}.wav"
        # From the repository root, where klattsch-cli finds the dictionary.
        code, output = run([str(cli), *wav["cli_args"], str(out)], cwd=ROOT)
        if code != 0:
            failures.append(f"klattsch-cli {name}: exit {code}: {output.strip()[-300:]}")
        elif Path(wav["path"]).read_bytes() != out.read_bytes():
            failures.append(f"save_wav {name}: differs from klattsch-cli")
        else:
            print(f"    save_wav {name}: same bytes as klattsch-cli")
    return failures


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--godot", type=Path, required=True, help="Godot 4.7.2 executable")
    ap.add_argument("--addon", type=Path, required=True, help="folder holding addons/klattsch (package_addon.py --out)")
    ap.add_argument("--cli", type=Path, help="the klattsch CLI (target/release/klattsch), to compare save_wav output")
    ap.add_argument("--work", type=Path, help="work folder (default: a temporary one)")
    args = ap.parse_args()

    work = args.work or Path(tempfile.mkdtemp(prefix="klattsch-godot-tests-"))
    work.mkdir(parents=True, exist_ok=True)
    project = make_project(args.addon.resolve(), work)
    godot = str(args.godot.resolve())
    print(f"project: {project}")

    # Import the project (and register the extension) before running it.
    code, output = run([godot, "--headless", "--path", str(project), "--import"])
    if code != 0:
        print(output[-3000:])
        print(f"FAIL import: exit {code}")
        return 1

    failures = []

    def check(name, cmd, tag, extra=None):
        code, output = run(cmd)
        r = result(output, tag)
        problems = []
        if r is None:
            problems.append(f"no {tag} line")
        elif not r.get("ok"):
            problems.append(f"failed: {r.get('failed', r.get('checks'))}")
        if code != 0:
            problems.append(f"exit {code}")
        if r is not None and extra:
            problems += extra(r)
        if problems:
            print(output[-4000:])
            print(f"FAIL {name}: {'; '.join(map(str, problems))}")
            failures.append(name)
        else:
            print(f"ok   {name}")

    for scene in SCENES:
        step = {"speech": 2, "exact": 3, "player_test": 4, "lang_test": 5, "sample_test": 6, "highlight_test": 8}[scene]
        extra = (lambda r: compare_wavs(r, args.cli.resolve(), work)) if scene == "sample_test" and args.cli else None
        check(scene, [godot, "--headless", "--path", str(project), f"res://{scene}.tscn"], f"STEP{step}_RESULT", extra)

    check("editor", [godot, "--headless", "--editor", "--path", str(project), "--", "--klattsch-editor-test"],
          "STEP8_EDITOR_RESULT")

    print(f"{len(SCENES) + 1 - len(failures)} of {len(SCENES) + 1} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
