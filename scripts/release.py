#!/usr/bin/env python3
"""Prepare and validate nvim-gpui release metadata."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import NoReturn


VERSION_PATTERN = re.compile(
    r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)
PACKAGE_SECTION_PATTERN = re.compile(
    r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)"
)
PACKAGE_VERSION_PATTERN = re.compile(r'(?m)^version\s*=\s*"([^"]+)"\s*$')
LOCK_PACKAGE_PATTERN = re.compile(
    r'(?ms)^\[\[package\]\]\nname = "nvim-gpui"\nversion = "([^"]+)"'
)


def fail(message: str) -> NoReturn:
    print(f"release metadata error: {message}", file=sys.stderr)
    raise SystemExit(1)


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        fail(f"cannot read {path}: {error}")


def write(path: Path, content: str) -> None:
    try:
        path.write_text(content, encoding="utf-8")
    except OSError as error:
        fail(f"cannot write {path}: {error}")


def package_version(cargo_toml: str) -> str:
    section = PACKAGE_SECTION_PATTERN.search(cargo_toml)
    if section is None:
        fail("Cargo.toml has no [package] section")

    version = PACKAGE_VERSION_PATTERN.search(section.group(1))
    if version is None:
        fail("Cargo.toml [package] section has no version")
    return version.group(1)


def replace_package_version(cargo_toml: str, version: str) -> str:
    section = PACKAGE_SECTION_PATTERN.search(cargo_toml)
    if section is None:
        fail("Cargo.toml has no [package] section")

    package_version_match = PACKAGE_VERSION_PATTERN.search(section.group(1))
    if package_version_match is None:
        fail("Cargo.toml [package] section has no version")

    start = section.start(1) + package_version_match.start(1)
    end = section.start(1) + package_version_match.end(1)
    return cargo_toml[:start] + version + cargo_toml[end:]


def replace_lock_version(cargo_lock: str, version: str) -> str:
    match = LOCK_PACKAGE_PATTERN.search(cargo_lock)
    if match is None:
        fail('Cargo.lock has no package entry for "nvim-gpui"')
    return cargo_lock[: match.start(1)] + version + cargo_lock[match.end(1) :]


def replace_one(path: Path, content: str, pattern: str, version: str) -> str:
    updated, count = re.subn(
        pattern,
        lambda match: f"{match.group(1)}{version}{match.group(2)}",
        content,
        count=1,
        flags=re.MULTILINE,
    )
    if count != 1:
        fail(f"{path} does not contain the expected version field")
    return updated


def metadata_files(root: Path) -> dict[Path, str]:
    return {
        root / "Cargo.toml": read(root / "Cargo.toml"),
        root / "Cargo.lock": read(root / "Cargo.lock"),
        root / "Casks" / "nvim-gpui.rb": read(root / "Casks" / "nvim-gpui.rb"),
        root / "packaging" / "macos" / "Info.plist": read(
            root / "packaging" / "macos" / "Info.plist"
        ),
    }


def synchronized_versions(root: Path) -> dict[Path, str]:
    files = metadata_files(root)
    versions = {
        root / "Cargo.toml": package_version(files[root / "Cargo.toml"]),
    }

    lock_match = LOCK_PACKAGE_PATTERN.search(files[root / "Cargo.lock"])
    if lock_match is None:
        fail('Cargo.lock has no package entry for "nvim-gpui"')
    versions[root / "Cargo.lock"] = lock_match.group(1)

    cask_match = re.search(
        r'(?m)^\s+version\s+"([^"]+)"\s*$',
        files[root / "Casks" / "nvim-gpui.rb"],
    )
    if cask_match is None:
        fail("Casks/nvim-gpui.rb has no version field")
    versions[root / "Casks" / "nvim-gpui.rb"] = cask_match.group(1)

    plist = files[root / "packaging" / "macos" / "Info.plist"]
    for key in ("CFBundleShortVersionString", "CFBundleVersion"):
        match = re.search(
            rf"(?ms)<key>{re.escape(key)}</key>\s*<string>([^<]+)</string>",
            plist,
        )
        if match is None:
            fail(f"Info.plist has no {key} field")
        versions[root / "packaging" / "macos" / f"Info.plist:{key}"] = match.group(1)

    return versions


def changelog_section(root: Path, version: str) -> str:
    changelog = read(root / "CHANGELOG.md")
    match = re.search(
        rf"(?ms)^## \[{re.escape(version)}\].*?(?=^## |\Z)",
        changelog,
    )
    if match is None:
        fail(f"CHANGELOG.md has no section for [{version}]")
    return match.group(0).strip()


def normalize_version(raw_version: str) -> str:
    version = raw_version[1:] if raw_version.startswith("v") else raw_version
    if not VERSION_PATTERN.fullmatch(version):
        fail(f"invalid version {raw_version!r}; expected SemVer such as 0.2.0")
    return version


def prepare(root: Path, raw_version: str) -> None:
    version = normalize_version(raw_version)
    files = metadata_files(root)
    updates = {
        root / "Cargo.toml": replace_package_version(
            files[root / "Cargo.toml"], version
        ),
        root / "Cargo.lock": replace_lock_version(files[root / "Cargo.lock"], version),
        root / "Casks" / "nvim-gpui.rb": replace_one(
            root / "Casks" / "nvim-gpui.rb",
            files[root / "Casks" / "nvim-gpui.rb"],
            r'(?m)^(\s+version\s+")[^"]+("\s*)$',
            version,
        ),
        root / "packaging" / "macos" / "Info.plist": files[
            root / "packaging" / "macos" / "Info.plist"
        ],
    }
    plist = updates[root / "packaging" / "macos" / "Info.plist"]
    for key in ("CFBundleShortVersionString", "CFBundleVersion"):
        plist = replace_one(
            root / "packaging" / "macos" / "Info.plist",
            plist,
            rf"(?ms)(<key>{re.escape(key)}</key>\s*<string>)[^<]+(</string>)",
            version,
        )
    updates[root / "packaging" / "macos" / "Info.plist"] = plist

    for path, content in updates.items():
        if content != files[path]:
            write(path, content)
            print(f"updated {path.relative_to(root)} to {version}")

    print(
        "Version metadata synchronized. Add or update the matching "
        "CHANGELOG.md section before running release-check."
    )


def check(root: Path, raw_tag: str | None) -> None:
    versions = synchronized_versions(root)
    unique_versions = set(versions.values())
    if len(unique_versions) != 1:
        details = ", ".join(
            f"{path.relative_to(root)}={version}" for path, version in versions.items()
        )
        fail(f"version metadata is out of sync: {details}")

    version = next(iter(unique_versions))
    normalize_version(version)
    changelog_section(root, version)

    if raw_tag:
        tag = raw_tag if raw_tag.startswith("v") else f"v{raw_tag}"
        if tag != f"v{version}":
            fail(f"tag {raw_tag!r} does not match package version {version}")

    suffix = f" for {raw_tag}" if raw_tag else ""
    print(f"release metadata is synchronized at {version}{suffix}")


def notes(root: Path, raw_tag: str) -> None:
    version = normalize_version(raw_tag)
    print(changelog_section(root, version))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Synchronize and validate nvim-gpui release metadata."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help=argparse.SUPPRESS,
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    prepare_parser = subparsers.add_parser(
        "prepare", help="sync release metadata to a version"
    )
    prepare_parser.add_argument("version")

    check_parser = subparsers.add_parser(
        "check", help="validate synchronized metadata and changelog"
    )
    check_parser.add_argument("tag", nargs="?", help="optional release tag")

    notes_parser = subparsers.add_parser(
        "notes", help="print the changelog section for a release tag"
    )
    notes_parser.add_argument("tag")

    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root = args.root.resolve()
    if args.command == "prepare":
        prepare(root, args.version)
    elif args.command == "check":
        check(root, args.tag or None)
    elif args.command == "notes":
        notes(root, args.tag)
    else:
        fail(f"unknown command {args.command!r}")


if __name__ == "__main__":
    main()
