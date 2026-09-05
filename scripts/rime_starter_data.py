#!/usr/bin/env python3
"""Create the small application-private Rime starter-data set."""

from __future__ import annotations

import argparse
import re
import shutil
import sys
from pathlib import Path
from typing import NoReturn

try:
    import tomllib
except ModuleNotFoundError as error:  # pragma: no cover - depends on host Python
    raise SystemExit("Rime starter-data tooling requires Python 3.11 or newer") from error


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "packaging" / "rime" / "starter-data.toml"


def fail(message: str) -> NoReturn:
    print(f"rime starter-data error: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_manifest() -> dict:
    try:
        with MANIFEST.open("rb") as stream:
            return tomllib.load(stream)
    except OSError as error:
        fail(f"cannot read {MANIFEST}: {error}")
    except tomllib.TOMLDecodeError as error:
        fail(f"invalid {MANIFEST}: {error}")


def locate_data_root(source: Path) -> Path:
    candidates = (source / "share" / "rime-data", source / "rime-data", source)
    for candidate in candidates:
        if candidate.is_dir():
            return candidate
    fail(f"starter data directory does not exist: {source}")


def restrict_default_schema(content: str, schema: str) -> str:
    lines = content.splitlines(keepends=True)
    start = next(
        (
            index
            for index, line in enumerate(lines)
            if re.fullmatch(r"schema_list:\s*\r?\n?", line)
        ),
        None,
    )
    if start is None:
        fail("default.yaml has no schema_list section")

    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if re.fullmatch(r"switcher:\s*\r?\n?", lines[index])
        ),
        None,
    )
    if end is None:
        fail("default.yaml schema_list has no switcher section")

    newline = "\r\n" if "\r\n" in content else "\n"
    replacement = [f"schema_list:{newline}", f"  - schema: {schema}{newline}"]
    return "".join(lines[:start] + replacement + lines[end:])


def prepare(source: Path, output: Path) -> None:
    manifest = load_manifest()
    starter = manifest.get("starter", {})
    schema = starter.get("schema")
    files = starter.get("files")
    if not isinstance(schema, str) or not schema:
        fail("starter manifest has no schema")
    if not isinstance(files, list) or not files or not all(
        isinstance(item, str) for item in files
    ):
        fail("starter manifest has no valid files list")

    source_root = locate_data_root(source.resolve())
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        fail(f"output must be a new or empty directory: {output}")
    output.mkdir(parents=True, exist_ok=True)

    for relative_name in files:
        source_file = source_root / relative_name
        if not source_file.is_file():
            fail(f"starter data file is missing: {source_file}")
        destination = output / relative_name
        destination.parent.mkdir(parents=True, exist_ok=True)
        if relative_name == "default.yaml":
            destination.write_text(
                restrict_default_schema(source_file.read_text(encoding="utf-8"), schema),
                encoding="utf-8",
            )
        else:
            shutil.copy2(source_file, destination)

    print(f"prepared Rime starter data: {output}")
    print(f"  schema: {schema}")
    print(f"  files: {len(files)}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path, help="Rime data package root")
    parser.add_argument("--output", required=True, type=Path, help="empty starter-data output")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    prepare(args.source, args.output)


if __name__ == "__main__":
    main()
