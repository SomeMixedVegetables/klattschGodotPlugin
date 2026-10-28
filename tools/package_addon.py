"""Assemble the Godot addon (addons/klattsch) from built libraries.

    python tools/package_addon.py --windows klattsch_godot.dll \
        --linux libklattsch_godot.so --macos libklattsch_godot.dylib \
        --out dist/klattsch-godot [--zip]

Copies the libraries under the names addon/klattsch.gdextension expects,
plus the CMU dictionary, icons, README, LICENSE and THIRD-PARTY-NOTICES.md.
A missing platform library is an error unless --allow-missing is given (for
local test builds). With --zip, also writes <out>.zip.
"""

import argparse
import shutil
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

LIBS = {
    "windows": "klattsch_godot.windows.x86_64.dll",
    "linux": "libklattsch_godot.linux.x86_64.so",
    "macos": "libklattsch_godot.macos.universal.dylib",
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for platform in LIBS:
        ap.add_argument(f"--{platform}", type=Path, help=f"built {platform} library")
    ap.add_argument("--out", type=Path, required=True, help="output folder (gets addons/klattsch inside)")
    ap.add_argument("--zip", action="store_true", help="also write <out>.zip")
    ap.add_argument("--allow-missing", action="store_true", help="skip platforms with no library")
    args = ap.parse_args()

    addon = args.out / "addons" / "klattsch"
    if args.out.exists():
        shutil.rmtree(args.out)
    (addon / "bin").mkdir(parents=True)

    for platform, name in LIBS.items():
        lib = getattr(args, platform)
        if lib is None:
            if not args.allow_missing:
                sys.exit(f"missing --{platform} library (use --allow-missing for a partial build)")
            print(f"skipping {platform}")
            continue
        if not lib.is_file():
            sys.exit(f"not a file: {lib}")
        shutil.copy2(lib, addon / "bin" / name)

    shutil.copy2(ROOT / "addon" / "klattsch.gdextension", addon)
    shutil.copy2(ROOT / "addon" / "README.md", addon)
    shutil.copytree(ROOT / "addon" / "icons", addon / "icons")
    shutil.copy2(ROOT / "data" / "cmudict" / "cmudict-0.7b", addon / "cmudict.dict")
    shutil.copy2(ROOT / "LICENSE", addon)
    shutil.copy2(ROOT / "THIRD-PARTY-NOTICES.md", addon)

    if args.zip:
        archive = args.out.with_suffix(".zip")
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
            for path in sorted(args.out.rglob("*")):
                if path.is_file():
                    z.write(path, path.relative_to(args.out).as_posix())
        print(f"wrote {archive}")
    for path in sorted(addon.rglob("*")):
        if path.is_file():
            print(f"  {path.relative_to(args.out).as_posix()}  {path.stat().st_size:,} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
