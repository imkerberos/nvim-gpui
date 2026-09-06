#!/usr/bin/env python3
"""Create the small application-private Rime starter-data set."""

from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import sys
import tarfile
from pathlib import Path, PurePosixPath, PureWindowsPath
from typing import NoReturn
from urllib.error import URLError
from urllib.request import Request, urlopen

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


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_relative_file(value: object, label: str) -> str:
    if not isinstance(value, str) or not value:
        fail(f"{label} must be a non-empty relative file path")
    posix_path = PurePosixPath(value)
    windows_path = PureWindowsPath(value)
    if (
        posix_path.is_absolute()
        or windows_path.is_absolute()
        or windows_path.drive
        or ".." in posix_path.parts
        or ".." in windows_path.parts
    ):
        fail(f"{label} must stay inside the extracted archive: {value}")
    return value


def validate_sha256(value: object, label: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        fail(f"{label} must be a lowercase SHA-256 digest")
    return value


def download_archive(archive: dict, cache_dir: Path) -> Path:
    name = archive["name"]
    digest = archive["sha256"]
    url = archive["url"]
    archive_dir = cache_dir / "archives"
    archive_dir.mkdir(parents=True, exist_ok=True)
    destination = archive_dir / f"{name}-{digest[:12]}.tar.gz"

    if destination.is_file():
        actual = sha256(destination)
        if actual != digest:
            fail(
                f"cached archive has the wrong SHA-256: {destination} "
                f"(expected {digest}, got {actual}); remove it and retry"
            )
        return destination

    partial = destination.with_name(destination.name + ".part")
    request = Request(url, headers={"User-Agent": "nvim-gpui-rime-starter-data"})
    try:
        with urlopen(request, timeout=120) as response, partial.open("wb") as stream:
            while chunk := response.read(1024 * 1024):
                stream.write(chunk)
        actual = sha256(partial)
        if actual != digest:
            fail(
                f"downloaded archive has the wrong SHA-256: {url} "
                f"(expected {digest}, got {actual})"
            )
        partial.replace(destination)
    except (OSError, URLError) as error:
        fail(f"could not download {url}: {error}")
    finally:
        partial.unlink(missing_ok=True)
    return destination


def extract_archive(archive: Path, name: str, digest: str, cache_dir: Path) -> Path:
    extraction_dir = cache_dir / "extracted" / f"{name}-{digest[:12]}"
    if extraction_dir.is_dir():
        roots = [item for item in extraction_dir.iterdir() if item.is_dir()]
        if len(roots) == 1:
            return roots[0]
        fail(f"cached archive extraction is incomplete: {extraction_dir}")
    if extraction_dir.exists():
        fail(f"cached archive extraction is not a directory: {extraction_dir}")

    extraction_dir.parent.mkdir(parents=True, exist_ok=True)
    partial_dir = extraction_dir.with_name(extraction_dir.name + ".part")
    if partial_dir.exists():
        shutil.rmtree(partial_dir)
    partial_dir.mkdir()
    try:
        try:
            with tarfile.open(archive, "r:*") as stream:
                members = stream.getmembers()
                for member in members:
                    posix_name = PurePosixPath(member.name)
                    windows_name = PureWindowsPath(member.name)
                    if (
                        posix_name.is_absolute()
                        or windows_name.is_absolute()
                        or windows_name.drive
                        or ".." in posix_name.parts
                        or ".." in windows_name.parts
                    ):
                        fail(f"archive contains an unsafe path: {member.name}")
                    if member.issym() or member.islnk():
                        fail(f"archive contains an unsupported link: {member.name}")
                    if not member.isdir() and not member.isfile():
                        fail(f"archive contains an unsupported entry: {member.name}")
                if sys.version_info >= (3, 12):
                    stream.extractall(partial_dir, filter="data")
                else:  # Python 3.11 has no extraction filter argument.
                    stream.extractall(partial_dir)
        except (OSError, tarfile.TarError) as error:
            fail(f"could not extract downloaded archive {archive}: {error}")
        roots = [item for item in partial_dir.iterdir() if item.is_dir()]
        if len(roots) != 1:
            fail(f"archive must contain one top-level directory: {archive}")
        partial_dir.replace(extraction_dir)
        return extraction_dir / roots[0].name
    finally:
        if partial_dir.exists():
            shutil.rmtree(partial_dir)


def download_starter_data(cache_dir: Path) -> Path:
    manifest = load_manifest()
    download = manifest.get("download", {})
    archives = download.get("archives") if isinstance(download, dict) else None
    cache_key = download.get("cache_key") if isinstance(download, dict) else None
    if not isinstance(archives, list) or not archives:
        fail("starter manifest has no download archives")
    if (
        not isinstance(cache_key, str)
        or re.fullmatch(r"[a-z0-9][a-z0-9._-]*", cache_key) is None
    ):
        fail("starter manifest has no download cache key")

    starter = manifest.get("starter", {})
    starter_files = starter.get("files") if isinstance(starter, dict) else None
    if not isinstance(starter_files, list):
        fail("starter manifest has no valid files list")
    required_files = {
        validate_relative_file(item, "starter file") for item in starter_files
    }
    assembled = cache_dir / cache_key
    assembled.mkdir(parents=True, exist_ok=True)

    copied_files: set[str] = set()
    for index, raw_archive in enumerate(archives):
        if not isinstance(raw_archive, dict):
            fail(f"download archive {index} is not a table")
        name = raw_archive.get("name")
        url = raw_archive.get("url")
        digest = raw_archive.get("sha256")
        files = raw_archive.get("files")
        if (
            not isinstance(name, str)
            or re.fullmatch(r"[a-z0-9][a-z0-9._-]*", name) is None
        ):
            fail(f"download archive {index} has an invalid name")
        if not isinstance(url, str) or not url.startswith("https://"):
            fail(f"download archive {name} must use an HTTPS URL")
        digest = validate_sha256(digest, f"download archive {name} SHA-256")
        if not isinstance(files, list) or not files:
            fail(f"download archive {name} has no files")
        files = [
            validate_relative_file(item, f"download archive {name} file")
            for item in files
        ]
        archive = download_archive(
            {"name": name, "url": url, "sha256": digest}, cache_dir
        )
        source_root = extract_archive(archive, name, digest, cache_dir)
        for relative_name in files:
            if relative_name in copied_files:
                fail(f"download file is listed more than once: {relative_name}")
            copied_files.add(relative_name)
            source_file = source_root / relative_name
            if not source_file.is_file():
                fail(f"download archive file is missing: {source_file}")
            destination = assembled / relative_name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_file, destination)

    missing = sorted(required_files - copied_files)
    if missing:
        fail(f"download manifest does not provide starter files: {', '.join(missing)}")
    return assembled


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
        destination.chmod(destination.stat().st_mode & ~0o222)

    print(f"prepared Rime starter data: {output}")
    print(f"  schema: {schema}")
    print(f"  files: {len(files)}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--source", type=Path, help="Rime data package root")
    source.add_argument(
        "--download",
        action="store_true",
        help="download the pinned official Rime data sources",
    )
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=ROOT / ".cache" / "rime-data",
        help="download and extraction cache directory",
    )
    parser.add_argument(
        "--output", required=True, type=Path, help="empty starter-data output"
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.source is None and not args.download:
        fail("pass either --source DIR or --download")
    source = (
        download_starter_data(args.cache_dir.resolve())
        if args.download
        else args.source
    )
    prepare(source, args.output)


if __name__ == "__main__":
    main()
