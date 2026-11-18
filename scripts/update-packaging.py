"""Sync WinGet manifests and AUR (jiyue-bin) PKGBUILD/.SRCINFO from a GitHub Release.

Run after the GitHub Release workflow for v<x.y.z> succeeds:
    python scripts/update-packaging.py <x.y.z>
"""

import argparse
import ctypes
import datetime as dt
import hashlib
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+$")


def parse_sha256sums(path: Path) -> dict[str, str]:
    sums: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.strip().split()
        if len(parts) == 2:
            digest, fname = parts
            sums[fname] = digest.lower()
    return sums


def read_msi_product_code(msi_path: Path) -> str:
    if sys.platform == "win32":
        msi = ctypes.windll.msi
        hdb = ctypes.c_void_p()
        if msi.MsiOpenDatabaseW(ctypes.c_wchar_p(str(msi_path)), ctypes.c_void_p(0), ctypes.byref(hdb)) != 0:
            raise RuntimeError(f"failed to open MSI database: {msi_path}")
        hview = ctypes.c_void_p()
        q = "SELECT `Value` FROM `Property` WHERE `Property`='ProductCode'"
        try:
            if msi.MsiDatabaseOpenViewW(hdb, ctypes.c_wchar_p(q), ctypes.byref(hview)) != 0:
                raise RuntimeError("MsiDatabaseOpenViewW failed")
            if msi.MsiViewExecute(hview, ctypes.c_void_p(0)) != 0:
                raise RuntimeError("MsiViewExecute failed")
            hrec = ctypes.c_void_p()
            if msi.MsiViewFetch(hview, ctypes.byref(hrec)) != 0:
                raise RuntimeError("ProductCode not found in MSI")
            try:
                buf = ctypes.create_unicode_buffer(128)
                sz = ctypes.c_uint(128)
                msi.MsiRecordGetStringW(hrec, 1, buf, ctypes.byref(sz))
                return buf.value
            finally:
                msi.MsiCloseHandle(hrec)
        finally:
            msi.MsiCloseHandle(hview)
            msi.MsiCloseHandle(hdb)
    out = subprocess.check_output(["msiinfo", "export", str(msi_path), "Property"], text=True)
    for line in out.splitlines():
        if line.startswith("ProductCode\t"):
            return line.split("\t", 1)[1].strip()
    raise RuntimeError(f"unable to extract ProductCode from {msi_path}")


def write_winget(
    ver: str,
    release_date: str,
    x64_sha: str,
    arm64_sha: str,
    x64_code: str,
    arm64_code: str,
) -> Path:
    out_dir = ROOT / "packaging" / "winget" / "manifests" / "l" / "limitcool" / "Jiyue" / ver
    out_dir.mkdir(parents=True, exist_ok=True)

    (out_dir / "limitcool.Jiyue.yaml").write_text(
        "# Created using winget manifest generator\n"
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.9.0.schema.json\n\n"
        "PackageIdentifier: limitcool.Jiyue\n"
        f"PackageVersion: {ver}\n"
        "DefaultLocale: en-US\n"
        "ManifestType: version\n"
        "ManifestVersion: 1.9.0\n",
        encoding="utf-8",
        newline="\n",
    )

    (out_dir / "limitcool.Jiyue.installer.yaml").write_text(
        "# Created using winget manifest generator\n"
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.9.0.schema.json\n\n"
        "PackageIdentifier: limitcool.Jiyue\n"
        f"PackageVersion: {ver}\n"
        "InstallerType: wix\n"
        "Scope: user\n"
        "InstallModes:\n"
        "  - interactive\n"
        "  - silent\n"
        "  - silentWithProgress\n"
        "UpgradeBehavior: install\n"
        "FileExtensions:\n"
        "  - md\n"
        "  - markdown\n"
        f"ReleaseDate: {release_date}\n"
        "Installers:\n"
        "  - Architecture: x64\n"
        f"    InstallerUrl: https://github.com/limitcool/jiyue/releases/download/v{ver}/Jiyue-{ver}-x64.msi\n"
        f"    InstallerSha256: {x64_sha.upper()}\n"
        f"    ProductCode: '{x64_code}'\n"
        "    AppsAndFeaturesEntries:\n"
        "      - DisplayName: 霁月 Jiyue\n"
        "        Publisher: limitcool\n"
        f"        DisplayVersion: {ver}\n"
        f"        ProductCode: '{x64_code}'\n"
        "        UpgradeCode: '{F48B69CA-0C87-4698-A62A-0D80DE358AAF}'\n"
        "  - Architecture: arm64\n"
        f"    InstallerUrl: https://github.com/limitcool/jiyue/releases/download/v{ver}/Jiyue-{ver}-arm64.msi\n"
        f"    InstallerSha256: {arm64_sha.upper()}\n"
        f"    ProductCode: '{arm64_code}'\n"
        "    AppsAndFeaturesEntries:\n"
        "      - DisplayName: 霁月 Jiyue\n"
        "        Publisher: limitcool\n"
        f"        DisplayVersion: {ver}\n"
        f"        ProductCode: '{arm64_code}'\n"
        "        UpgradeCode: '{F48B69CA-0C87-4698-A62A-0D80DE358AAF}'\n"
        "ManifestType: installer\n"
        "ManifestVersion: 1.9.0\n",
        encoding="utf-8",
        newline="\n",
    )

    (out_dir / "limitcool.Jiyue.locale.en-US.yaml").write_text(
        "# Created using winget manifest generator\n"
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.9.0.schema.json\n\n"
        "PackageIdentifier: limitcool.Jiyue\n"
        f"PackageVersion: {ver}\n"
        "PackageLocale: en-US\n"
        "Publisher: limitcool\n"
        "PublisherUrl: https://github.com/limitcool\n"
        "PublisherSupportUrl: https://github.com/limitcool/jiyue/issues\n"
        "Author: limitcool\n"
        "PackageName: Jiyue\n"
        "PackageUrl: https://github.com/limitcool/jiyue\n"
        "License: Apache-2.0 OR MIT\n"
        "LicenseUrl: https://github.com/limitcool/jiyue/blob/main/LICENSE-MIT\n"
        "Copyright: Copyright (c) limitcool\n"
        "ShortDescription: Local-first Markdown note-taking app with wikilinks, graph view, and S3/WebDAV sync.\n"
        "Description: |-\n"
        "  Jiyue is a fast, local-first Markdown note-taking app built with Rust and GPUI.\n"
        "  Notes stay as plain Markdown files inside normal folders on disk, with live preview,\n"
        "  wikilinks, backlinks, global and local graph views, full-text Chinese/English search,\n"
        "  S3 and WebDAV sync with three-way merge, and an optional BYOK AI assistant.\n"
        "Moniker: jiyue\n"
        "Tags:\n"
        "  - markdown\n"
        "  - notes\n"
        "  - wikilink\n"
        "  - knowledge-base\n"
        "  - pkm\n"
        "  - rust\n"
        f"ReleaseNotesUrl: https://github.com/limitcool/jiyue/releases/tag/v{ver}\n"
        "Documentations:\n"
        "  - DocumentLabel: Documentation\n"
        "    DocumentUrl: https://github.com/limitcool/jiyue#readme\n"
        "ManifestType: defaultLocale\n"
        "ManifestVersion: 1.9.0\n",
        encoding="utf-8",
        newline="\n",
    )

    (out_dir / "limitcool.Jiyue.locale.zh-CN.yaml").write_text(
        "# Created using winget manifest generator\n"
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.locale.1.9.0.schema.json\n\n"
        "PackageIdentifier: limitcool.Jiyue\n"
        f"PackageVersion: {ver}\n"
        "PackageLocale: zh-CN\n"
        "Publisher: limitcool\n"
        "PublisherUrl: https://github.com/limitcool\n"
        "PublisherSupportUrl: https://github.com/limitcool/jiyue/issues\n"
        "Author: limitcool\n"
        "PackageName: 霁月 Jiyue\n"
        "PackageUrl: https://github.com/limitcool/jiyue\n"
        "License: Apache-2.0 OR MIT\n"
        "LicenseUrl: https://github.com/limitcool/jiyue/blob/main/LICENSE-MIT\n"
        "ShortDescription: 纯本地 Markdown 双链笔记应用，支持实时预览、知识图谱与原生 S3/WebDAV 同步。\n"
        "Description: |-\n"
        "  霁月 (Jiyue) 是一款基于 Rust 与 GPUI 构建的极简、本地优先 Markdown 双链笔记应用。\n"
        "  所有笔记均保存为硬盘上的普通 Markdown 文件，支持实时预览、悬浮格式工具条、双链与反链、\n"
        "  全局与局部知识图谱、支持中文分词的全文检索、S3/WebDAV 三向合并同步与自选密钥 AI 助手。\n"
        "Tags:\n"
        "  - markdown\n"
        "  - 笔记\n"
        "  - 双链\n"
        "  - 知识库\n"
        "  - rust\n"
        f"ReleaseNotesUrl: https://github.com/limitcool/jiyue/releases/tag/v{ver}\n"
        "ManifestType: locale\n"
        "ManifestVersion: 1.9.0\n",
        encoding="utf-8",
        newline="\n",
    )
    return out_dir


def update_aur(ver: str, lic_sha: str, amd64_sha: str, arm64_sha: str) -> None:
    aur_dir = ROOT / "packaging" / "aur" / "jiyue-bin"
    aur_dir.mkdir(parents=True, exist_ok=True)

    (aur_dir / "PKGBUILD").write_text(
        "# Maintainer: limitcool <initcoool@gmail.com>\n\n"
        "pkgname=jiyue-bin\n"
        f"pkgver={ver}\n"
        "pkgrel=1\n"
        "pkgdesc='Local-first Markdown note-taking app with wikilinks, graph view, and S3/WebDAV sync'\n"
        "arch=('x86_64' 'aarch64')\n"
        "url='https://github.com/limitcool/jiyue'\n"
        "license=('MIT OR Apache-2.0')\n"
        "depends=(\n"
        "    'glibc'\n"
        "    'gcc-libs'\n"
        "    'libxcb'\n"
        "    'libxkbcommon'\n"
        "    'libxkbcommon-x11'\n"
        "    'fontconfig'\n"
        "    'wayland'\n"
        "    'vulkan-icd-loader'\n"
        "    'hicolor-icon-theme'\n"
        ")\n"
        "optdepends=(\n"
        "    'vulkan-driver: GPU-accelerated rendering'\n"
        "    'alsa-lib: audio support'\n"
        ")\n"
        "provides=('jiyue')\n"
        "conflicts=('jiyue')\n"
        "options=('!strip')\n\n"
        'source=("LICENSE-MIT-${pkgver}::https://raw.githubusercontent.com/limitcool/jiyue/v${pkgver}/LICENSE-MIT")\n'
        'source_x86_64=("${pkgname}-${pkgver}-x86_64.deb::https://github.com/limitcool/jiyue/releases/download/v${pkgver}/jiyue_${pkgver}_amd64.deb")\n'
        'source_aarch64=("${pkgname}-${pkgver}-aarch64.deb::https://github.com/limitcool/jiyue/releases/download/v${pkgver}/jiyue_${pkgver}_arm64.deb")\n\n'
        f"sha256sums=('{lic_sha}')\n"
        f"sha256sums_x86_64=('{amd64_sha}')\n"
        f"sha256sums_aarch64=('{arm64_sha}')\n\n"
        "package() {\n"
        '    bsdtar -xf "${srcdir}/data.tar."* -C "${pkgdir}"\n'
        '    install -Dm644 "${srcdir}/LICENSE-MIT-${pkgver}" "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE-MIT"\n'
        "}\n",
        encoding="utf-8",
        newline="\n",
    )

    (aur_dir / ".SRCINFO").write_text(
        "pkgbase = jiyue-bin\n"
        "\tpkgdesc = Local-first Markdown note-taking app with wikilinks, graph view, and S3/WebDAV sync\n"
        f"\tpkgver = {ver}\n"
        "\tpkgrel = 1\n"
        "\turl = https://github.com/limitcool/jiyue\n"
        "\tarch = x86_64\n"
        "\tarch = aarch64\n"
        "\tlicense = MIT OR Apache-2.0\n"
        "\tdepends = glibc\n"
        "\tdepends = gcc-libs\n"
        "\tdepends = libxcb\n"
        "\tdepends = libxkbcommon\n"
        "\tdepends = libxkbcommon-x11\n"
        "\tdepends = fontconfig\n"
        "\tdepends = wayland\n"
        "\tdepends = vulkan-icd-loader\n"
        "\tdepends = hicolor-icon-theme\n"
        "\toptdepends = vulkan-driver: GPU-accelerated rendering\n"
        "\toptdepends = alsa-lib: audio support\n"
        "\tprovides = jiyue\n"
        "\tconflicts = jiyue\n"
        "\toptions = !strip\n"
        f"\tsource = LICENSE-MIT-{ver}::https://raw.githubusercontent.com/limitcool/jiyue/v{ver}/LICENSE-MIT\n"
        f"\tsha256sums = {lic_sha}\n"
        f"\tsource_x86_64 = jiyue-bin-{ver}-x86_64.deb::https://github.com/limitcool/jiyue/releases/download/v{ver}/jiyue_{ver}_amd64.deb\n"
        f"\tsha256sums_x86_64 = {amd64_sha}\n"
        f"\tsource_aarch64 = jiyue-bin-{ver}-aarch64.deb::https://github.com/limitcool/jiyue/releases/download/v{ver}/jiyue_{ver}_arm64.deb\n"
        f"\tsha256sums_aarch64 = {arm64_sha}\n\n"
        "pkgname = jiyue-bin\n",
        encoding="utf-8",
        newline="\n",
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("version", help="Release version (e.g. 0.4.0)")
    parser.add_argument("--date", default=dt.date.today().isoformat(), help="Release date YYYY-MM-DD")
    args = parser.parse_args()

    ver = args.version.removeprefix("v").strip()
    if not SEMVER_RE.match(ver):
        print(f"invalid version '{ver}', expected x.y.z", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory() as tmp:
        tmp_dir = Path(tmp)
        subprocess.run(
            [
                "gh",
                "release",
                "download",
                f"v{ver}",
                "-p",
                "SHA256SUMS",
                "-p",
                f"Jiyue-{ver}-x64.msi",
                "-p",
                f"Jiyue-{ver}-arm64.msi",
                "-D",
                str(tmp_dir),
                "--clobber",
            ],
            check=True,
        )
        sums = parse_sha256sums(tmp_dir / "SHA256SUMS")
        x64_code = read_msi_product_code(tmp_dir / f"Jiyue-{ver}-x64.msi")
        arm64_code = read_msi_product_code(tmp_dir / f"Jiyue-{ver}-arm64.msi")

    lic_sha = hashlib.sha256((ROOT / "LICENSE-MIT").read_bytes()).hexdigest()
    winget_dir = write_winget(
        ver,
        args.date,
        sums[f"Jiyue-{ver}-x64.msi"],
        sums[f"Jiyue-{ver}-arm64.msi"],
        x64_code,
        arm64_code,
    )
    update_aur(
        ver,
        lic_sha,
        sums[f"jiyue_{ver}_amd64.deb"],
        sums[f"jiyue_{ver}_arm64.deb"],
    )
    print(f"Updated WinGet manifests: {winget_dir}")
    print(f"Updated AUR files: {ROOT / 'packaging' / 'aur' / 'jiyue-bin'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
