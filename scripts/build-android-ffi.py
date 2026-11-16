#!/usr/bin/env python3
"""Cross-compile `crates/jiyue-ffi` into Android `jniLibs` without external wrappers.

Usage:
    python3 scripts/build-android-ffi.py
    python3 scripts/build-android-ffi.py --targets arm64-v8a armeabi-v7a x86_64
"""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

ABI_SPECS = {
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android"),
    "armeabi-v7a": ("armv7-linux-androideabi", "armv7a-linux-androideabi"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android"),
}


def find_ndk_dir() -> Path:
    for var in ("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "NDK_HOME"):
        val = os.environ.get(var, "").strip()
        if val and Path(val).is_dir():
            return Path(val)

    sdk_candidates: list[Path] = []
    for var in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        val = os.environ.get(var, "").strip()
        if val:
            sdk_candidates.append(Path(val))

    if platform.system() == "Windows":
        local_app = os.environ.get("LOCALAPPDATA", "")
        if local_app:
            sdk_candidates.append(Path(local_app) / "Android" / "Sdk")
    else:
        sdk_candidates.append(Path.home() / "Android" / "Sdk")
        sdk_candidates.append(Path.home() / "Library" / "Android" / "sdk")

    for sdk in sdk_candidates:
        ndk_root = sdk / "ndk"
        if ndk_root.is_dir():
            versions = sorted((p for p in ndk_root.iterdir() if p.is_dir()), reverse=True)
            if versions:
                return versions[0]

    raise RuntimeError(
        "Android NDK not found. Set ANDROID_NDK_HOME or install NDK under Android SDK."
    )


def find_llvm_bin(ndk_dir: Path) -> Path:
    prebuilt = ndk_dir / "toolchains" / "llvm" / "prebuilt"
    if not prebuilt.is_dir():
        raise RuntimeError(f"LLVM prebuilt directory missing in NDK: {prebuilt}")

    candidates = [p for p in prebuilt.iterdir() if p.is_dir()]
    if not candidates:
        raise RuntimeError(f"No host toolchain directories inside {prebuilt}")
    bin_dir = candidates[0] / "bin"
    if not bin_dir.is_dir():
        raise RuntimeError(f"NDK bin directory missing: {bin_dir}")
    return bin_dir


def build_abi(
    repo_root: Path,
    ndk_dir: Path,
    llvm_bin: Path,
    abi: str,
    api_level: int,
    out_root: Path,
    locked: bool,
) -> Path:
    triple, clang_prefix = ABI_SPECS[abi]
    is_win = platform.system() == "Windows"
    cmd_suffix = ".cmd" if is_win else ""
    exe_suffix = ".exe" if is_win else ""

    clang = llvm_bin / f"{clang_prefix}{api_level}-clang{cmd_suffix}"
    clangxx = llvm_bin / f"{clang_prefix}{api_level}-clang++{cmd_suffix}"
    ar = llvm_bin / f"llvm-ar{exe_suffix}"

    for tool in (clang, clangxx, ar):
        if not tool.exists():
            raise RuntimeError(f"Required NDK tool missing: {tool}")

    env = os.environ.copy()
    env["ANDROID_NDK_HOME"] = str(ndk_dir)
    env["ANDROID_NDK_ROOT"] = str(ndk_dir)
    env["ANDROID_NDK"] = str(ndk_dir)
    norm_triple = triple.replace("-", "_")
    upper_triple = norm_triple.upper()
    env[f"CC_{norm_triple}"] = str(clang)
    env[f"CXX_{norm_triple}"] = str(clangxx)
    env[f"AR_{norm_triple}"] = str(ar)
    env[f"CARGO_TARGET_{upper_triple}_LINKER"] = str(clang)

    # Ensure target toolchain is installed in rustup when rustup is available.
    if shutil.which("rustup"):
        installed = subprocess.check_output(
            ["rustup", "target", "list", "--installed"],
            cwd=repo_root,
            text=True,
        ).splitlines()
        if triple not in {t.strip() for t in installed}:
            print(f"[jiyue-ffi] Installing rustup target {triple}...", flush=True)
            subprocess.check_call(["rustup", "target", "add", triple], cwd=repo_root)

    cmd = [
        "cargo",
        "build",
        "--release",
        "-p",
        "jiyue-ffi",
        "--target",
        triple,
    ]
    if locked:
        cmd.append("--locked")

    print(f"[jiyue-ffi] Building {abi} ({triple}, API {api_level})...", flush=True)
    subprocess.check_call(cmd, cwd=repo_root, env=env)

    built_so = repo_root / "target" / triple / "release" / "libjiyue_ffi.so"
    if not built_so.is_file():
        raise RuntimeError(f"Expected shared library not produced: {built_so}")

    dest_dir = out_root / abi
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest_so = dest_dir / "libjiyue_ffi.so"
    shutil.copy2(built_so, dest_so)
    print(f"[jiyue-ffi] Copied {dest_so} ({dest_so.stat().st_size} bytes)", flush=True)
    return dest_so


def main() -> int:
    parser = argparse.ArgumentParser(description="Build jiyue-ffi Android jniLibs.")
    parser.add_argument(
        "--targets",
        nargs="+",
        choices=sorted(ABI_SPECS.keys()),
        default=["arm64-v8a", "armeabi-v7a", "x86_64"],
        help="Android ABIs to build (default: all three).",
    )
    parser.add_argument(
        "--api-level",
        type=int,
        default=21,
        help="Android NDK API level for clang wrapper (default: 21).",
    )
    parser.add_argument(
        "--locked",
        action="store_true",
        help="Pass --locked to cargo build.",
    )
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    out_root = repo_root / "mobile" / "android" / "app" / "src" / "main" / "jniLibs"

    ndk_dir = find_ndk_dir()
    llvm_bin = find_llvm_bin(ndk_dir)
    print(f"[jiyue-ffi] Using Android NDK: {ndk_dir}", flush=True)

    for abi in args.targets:
        build_abi(
            repo_root=repo_root,
            ndk_dir=ndk_dir,
            llvm_bin=llvm_bin,
            abi=abi,
            api_level=args.api_level,
            out_root=out_root,
            locked=args.locked,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
