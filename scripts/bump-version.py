"""Synchronize the release version across Cargo workspace, Cargo.lock, and Flutter mobile.

Run: python scripts/bump-version.py <x.y.z>
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+$")


def read_text_preserve_newlines(path: Path) -> str:
    with path.open("r", encoding="utf-8", newline="") as f:
        return f.read()


def write_text_preserve_newlines(path: Path, text: str) -> None:
    with path.open("w", encoding="utf-8", newline="") as f:
        f.write(text)


def update_workspace_toml(version: str) -> None:
    path = ROOT / "Cargo.toml"
    original = read_text_preserve_newlines(path)
    lines = original.splitlines(keepends=True)
    in_section = False
    replaced = False
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("["):
            in_section = stripped == "[workspace.package]"
            continue
        if in_section and stripped.startswith("version ="):
            ending = "\r\n" if line.endswith("\r\n") else "\n"
            lines[idx] = f'version = "{version}"{ending}'
            replaced = True
            break
    if not replaced:
        raise RuntimeError("missing version key under [workspace.package] in Cargo.toml")
    updated = "".join(lines)
    if updated != original:
        write_text_preserve_newlines(path, updated)


def update_pubspec(version: str) -> str:
    path = ROOT / "mobile" / "pubspec.yaml"
    original = read_text_preserve_newlines(path)
    pattern = re.compile(r"^version:\s*(\d+\.\d+\.\d+)\+(\d+)(\r?\n?)$", re.MULTILINE)
    match = pattern.search(original)
    if not match:
        raise RuntimeError("missing version: x.y.z+N in mobile/pubspec.yaml")
    old_ver, old_code, ending = match.group(1), int(match.group(2)), match.group(3)
    new_code = old_code if old_ver == version else old_code + 1
    updated = pattern.sub(f"version: {version}+{new_code}{ending}", original, count=1)
    if updated != original:
        write_text_preserve_newlines(path, updated)
    return f"{version}+{new_code}"


def update_dart_fallback(version: str) -> None:
    path = ROOT / "mobile" / "lib" / "services" / "update_service.dart"
    original = read_text_preserve_newlines(path)
    pattern = re.compile(r"const String currentAppVersion = '([^']+)';")
    if not pattern.search(original):
        raise RuntimeError("missing currentAppVersion constant in update_service.dart")
    updated = pattern.sub(f"const String currentAppVersion = '{version}';", original, count=1)
    if updated != original:
        write_text_preserve_newlines(path, updated)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python scripts/bump-version.py <x.y.z>", file=sys.stderr)
        return 2
    version = sys.argv[1].removeprefix("v").strip()
    if not SEMVER_RE.match(version):
        print(f"invalid semantic version '{version}', expected x.y.z", file=sys.stderr)
        return 2

    update_workspace_toml(version)
    mobile_ver = update_pubspec(version)
    update_dart_fallback(version)
    subprocess.run(["cargo", "update", "--workspace"], cwd=ROOT, check=True)
    print(f"Updated version: workspace={version}, mobile={mobile_ver}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
