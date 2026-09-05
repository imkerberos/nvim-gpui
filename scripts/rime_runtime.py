#!/usr/bin/env python3
"""Stage and validate the application-private librime runtime layout."""

from __future__ import annotations

import argparse
import os
import shutil
import stat
import sys
from pathlib import Path
from typing import NoReturn

try:
    import tomllib
except ModuleNotFoundError as error:  # pragma: no cover - depends on host Python
    raise SystemExit("rime runtime tooling requires Python 3.11 or newer") from error


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "packaging" / "rime" / "runtime.toml"
NIX_MARKERS = (b"/nix/store/", b"\\nix\\store\\")


def fail(message: str) -> NoReturn:
    print(f"rime runtime error: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_manifest() -> dict:
    try:
        with MANIFEST.open("rb") as stream:
            return tomllib.load(stream)
    except OSError as error:
        fail(f"cannot read {MANIFEST}: {error}")
    except tomllib.TOMLDecodeError as error:
        fail(f"invalid {MANIFEST}: {error}")


def target_name(value: str | None) -> str:
    if value is not None:
        if value not in {"macos", "windows", "linux"}:
            fail(f"unsupported target {value!r}")
        return value
    if sys.platform == "darwin":
        return "macos"
    if os.name == "nt":
        return "windows"
    return "linux"


def runtime_files(root: Path):
    for path in root.rglob("*"):
        if path.is_file():
            yield path


def remove_runtime_output(path: Path) -> None:
    """Remove a previous staged output, including read-only data trees."""
    if not path.exists():
        return

    # Shared data is often installed read-only (for example, from a package
    # store). The output is an exact staging target owned by this command, so
    # make only that tree user-writable before replacing it.
    for current, directories, files in os.walk(path, topdown=False):
        for name in files + directories:
            child = Path(current) / name
            if child.is_symlink():
                continue
            try:
                child.chmod(child.stat().st_mode | stat.S_IWUSR)
            except OSError as error:
                fail(f"cannot make old runtime output writable {child}: {error}")
        current_path = Path(current)
        try:
            current_path.chmod(current_path.stat().st_mode | stat.S_IWUSR)
        except OSError as error:
            fail(f"cannot make old runtime directory writable {current_path}: {error}")
    shutil.rmtree(path)


def validate(root: Path, platform: str, require_data: bool) -> list[Path]:
    manifest = load_manifest()
    layout = manifest["layout"]
    platform_manifest = manifest["platform"][platform]

    if not root.is_dir():
        fail(f"runtime root does not exist or is not a directory: {root}")

    library_dir = root / layout["library_directory"]
    data_dir = root / layout["data_directory"]
    modules_dir = root / layout["modules_directory"]
    for directory, label in (
        (library_dir, "library"),
        (data_dir, "data"),
    ):
        if not directory.is_dir():
            fail(f"runtime {label} directory is missing: {directory}")

    library_names = platform_manifest["library_names"]
    libraries = [library_dir / name for name in library_names if (library_dir / name).is_file()]
    if not libraries:
        expected = ", ".join(str(library_dir / name) for name in library_names)
        fail(f"no librime library found; expected one of: {expected}")

    data_files = list(runtime_files(data_dir))
    if require_data and not data_files:
        fail(f"runtime data directory is empty: {data_dir}")

    for path in runtime_files(root):
        try:
            contents = path.read_bytes()
        except OSError as error:
            fail(f"cannot read runtime file {path}: {error}")
        if any(marker in contents for marker in NIX_MARKERS):
            fail(f"runtime file contains a Nix store reference: {path}")

    if modules_dir.exists() and not modules_dir.is_dir():
        fail(f"runtime modules path is not a directory: {modules_dir}")

    return libraries


def check(args: argparse.Namespace) -> None:
    platform = target_name(args.platform)
    libraries = validate(Path(args.root).resolve(), platform, args.require_data)
    print(f"verified Rime runtime: {Path(args.root).resolve()}")
    print(f"  target: {platform}")
    print(f"  library: {libraries[0].name}")


def stage(args: argparse.Namespace) -> None:
    platform = target_name(args.platform)
    source = Path(args.source).resolve()
    output = Path(args.output).resolve()
    validate(source, platform, require_data=True)

    if source == output or source in output.parents or output in source.parents:
        fail("runtime source and output must be separate directories")

    if output.exists():
        if output.is_dir():
            remove_runtime_output(output)
        else:
            fail(f"runtime output exists and is not a directory: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    # Preserve versioned-library symlinks (for example, librime.dylib and
    # librime.1.dylib) instead of expanding each link into another full copy.
    # The runtime artifact is copied as a layout, not as a dereferenced data
    # snapshot.
    shutil.copytree(source, output, symlinks=True)
    validate(output, platform, require_data=True)
    print(f"staged Rime runtime: {output}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    check_parser = subparsers.add_parser("check", help="validate a staged runtime")
    check_parser.add_argument("--root", required=True, help="staged runtime root")
    check_parser.add_argument("--platform", choices=("macos", "windows", "linux"))
    check_parser.add_argument(
        "--require-data",
        action="store_true",
        help="require at least one file in the shared data directory",
    )
    check_parser.set_defaults(handler=check)

    stage_parser = subparsers.add_parser("stage", help="copy and validate a runtime")
    stage_parser.add_argument("--source", required=True, help="source runtime root")
    stage_parser.add_argument("--output", required=True, help="staged runtime root")
    stage_parser.add_argument("--platform", choices=("macos", "windows", "linux"))
    stage_parser.set_defaults(handler=stage)

    return parser.parse_args()


def main() -> None:
    args = parse_args()
    args.handler(args)


if __name__ == "__main__":
    main()
