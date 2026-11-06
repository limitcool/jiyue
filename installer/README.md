# Jiyue installer

This folder builds the Windows installer (MSI, WiX Toolset v6) and a portable zip of Jiyue.
The same script runs on a developer PC and on GitHub's `windows-latest` runner (`release.yml`).

## Outputs

`build.ps1` writes to `installer\out\` (or to `-OutDir`):

| File | Content |
| --- | --- |
| `Jiyue-<version>-x64.msi` | Per-user installer (no administrator rights). Default UI language zh-CN. |
| `Jiyue-<version>-x64-portable.zip` | `jiyue.exe`, `jiyue-mcp.exe`, a short `README.txt`, the licence files and `THIRD_PARTY_NOTICES.md`. No registry entries, no shortcuts. |
| `Jiyue-<version>-arm64.msi` | The same installer for ARM64 Windows (`-Arch arm64`, MSI platform `Arm64`). Only ARM64 Windows can install it. |
| `Jiyue-<version>-arm64-portable.zip` | The same zip with the ARM64 binaries. |

`-Arch arm64` builds `aarch64-pc-windows-msvc` with `--target`, so its binaries are in
`<target>\aarch64-pc-windows-msvc\release`. The script adds the Rust target when `rustup` is present. The
x64 outputs and their names are unchanged.

The version is read from `crates\jiyue-app\Cargo.toml` (`[package] version`).

## Build

Requirements: PowerShell 7, Rust (cargo, with the toolchain from `rust-toolchain.toml`), the .NET SDK
8 or newer (for the local WiX tool), and Python 3 (standard library only, for the notices). Nothing is
installed globally, and the repository can be in any folder.

```powershell
cd <repo>\installer
.\build.ps1                                            # release build, MSI, validation and zip
.\build.ps1 -SkipBuild                                 # package the binaries already in the target folder
.\build.ps1 -SkipBuild -NoZip -OutDir C:\temp\jiyue    # MSI only, into another folder
```

Parameters:

| Parameter | Meaning |
| --- | --- |
| `-SkipBuild` (alias `-SkipCargo`) | Do not run cargo; package the release binaries already in the target folder. |
| `-Arch x64\|arm64` | Target architecture. Default `x64`. |
| `-OutDir <path>` | Output folder. Default `installer\out`. |
| `-NoZip` | Build only the MSI. |
| `-Python <path>` | Python 3 for `scripts\gen-notices.py`. |

Environment variables:

| Variable | Meaning |
| --- | --- |
| `CARGO_TARGET_DIR` | Target folder of the release build. Default `target-dist` (git-ignored, separate from the `target` development builds). |
| `JIYUE_PYTHON` | Python 3 interpreter, used when `-Python` is not given. |
| `JIYUE_SIGN_*` | Optional code signing, see below. |

Python is chosen in this order: `-Python`, `JIYUE_PYTHON`, then the first working one of `python`,
`py -3` and `python3` on PATH (the Windows Store alias, which does not run Python, is skipped). The script stops with a clear message when no Python 3 is found.

- Notes:
  - The script stops with a non-zero exit code on the first error. It never prompts and never prints a password.
  - The release build runs with `--locked`: an out-of-date `Cargo.lock` fails the build.
  - `build.ps1` prints the size and SHA-256 of every output at the end.
  - The MSI differs between two builds even from the same binaries (WiX generates a new product code per build), so compare the version, not the hash, across builds.

### WiX tool and extensions

- WiX is a local tool pinned in `.config\dotnet-tools.json` (`wix` 6.0.0). The build runs `dotnet tool restore`
  in this folder, so nothing is installed globally.
- The extensions `WixToolset.UI.wixext` and `WixToolset.Util.wixext` (the same version as the tool, read from
  the manifest) are cached in `installer\.wix\` (git-ignored). On every build the script lists the cache and adds
  only the missing extensions. On a clean machine or a fresh CI runner this needs network access once.

## Continuous integration

The `windows` job of `.github\workflows\release.yml` runs once per architecture: `windows-latest` with
`-Arch x64`, and `windows-11-arm` with `-Arch arm64` when the ARM64 entries are on (see `docs/ci.md`). Each job does:

1. checkout;
2. `actions/setup-python@v7` (Python 3.12), `actions/setup-dotnet@v6` (8.0 and 9.0);
3. `dotnet tool restore` in `installer\`;
4. the Rust toolchain and cache (as the other jobs);
5. optional signing: the PFX secret is decoded to a temporary file and its path is passed as `JIYUE_SIGN_PFX`;
6. `./installer/build.ps1 -Arch <x64|arm64>`, with `JIYUE_PYTHON` set to the Python from step 2;
7. upload of `installer\out\Jiyue-*-<arch>.msi` and `Jiyue-*-<arch>-portable.zip` as `release-windows-<arch>`.

The job needs no drive other than the checkout, no pre-made virtual environment and no path that is specific
to a developer PC. The tools it uses are on the runner: cargo, dotnet, Python and `signtool.exe` (Windows SDK, only when signing).

## Files

| File | Purpose |
| --- | --- |
| `Package.wxs` | The WiX package (per-user, features, shortcuts, registry, PATH, UI). |
| `zh-CN.wxl` | Simplified Chinese strings of the product. |
| `portable-README.txt` | README shipped in the portable zip (Chinese). |
| `build.ps1` | Notices, build, sign (optional), MSI, validation, zip, summary. |
| `.config\dotnet-tools.json` | Local WiX tool manifest. |
| `.wix\` (created by the build, git-ignored) | Cache of the WiX extensions. |
| `out\` (created by the build, git-ignored) | Outputs and intermediate files. |

## What the MSI installs

- Install folder: `%LOCALAPPDATA%\Programs\Jiyue` (per user, no admin). The folder can be changed on the command line with `INSTALLFOLDER=...`.
- Always installed (feature `Core`): `jiyue.exe`, `jiyue-mcp.exe`, `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md`, a Start Menu shortcut "霁月" (vault `%USERPROFILE%\Documents\Jiyue`).
- Optional features, off by default:
  - `DesktopShortcut`: a desktop shortcut.
  - `McpPath`: adds the install folder to the user `PATH`, so `jiyue-mcp` runs from any terminal.
  - `MarkdownVerb`: "用霁月打开" in the right-click menu of `.md` files and in "Open with". It does not change the default program of `.md` files.
- Uninstall removes everything the installer put in place. It never touches vault folders (`Documents\Jiyue` is created by the app on first run, not by the installer).
- Not removed by uninstall: the app's own cache folder `%LOCALAPPDATA%\Jiyue\cache` (created at run time) and the `.jiyue` folders inside your vaults. Delete them by hand if you want them gone.

### Choosing optional features

The stock WiX UI used here (`WixUI_Minimal`) has no feature page, so the optional features are chosen on the command line:

```powershell
msiexec /i Jiyue-0.1.0-x64.msi ADDLOCAL=Core,DesktopShortcut,McpPath,MarkdownVerb
msiexec /i Jiyue-0.1.0-x64.msi ADDLOCAL=ALL          # everything
```

The stock feature-tree UI (`WixUI_FeatureTree`) would show the checkboxes, but it always includes a licence page. The project now has a licence (MIT OR Apache-2.0), so that page is possible; it is not added yet.

## Verification

Done on the development PC (Windows 11, per-user):

- Earlier: `wix build` succeeds with no errors; `wix msi validate` (ICE) reports no errors. Two ICE91 warnings remain: the files are per-user by design, so ICE91 (per-machine copy) does not apply. (The current build reports five ICE91 warnings, one per file in the per-user folder; the same reason applies.)
- Earlier: silent install with all features, then silent uninstall (`msiexec /i ... /qn INSTALLFOLDER=<temp>` and `msiexec /x <msi> /qn`): files, Start Menu and desktop shortcuts, the registry values and the PATH entry were created, and all of them were removed again.
- Earlier: silent default install (no optional features): only the program files and the Start Menu shortcut appear; uninstall leaves nothing behind.
- Portability of the script: `-SkipBuild` and the full build (`cargo build --release --locked`) pass; a clean WiX extension cache is refilled by the build; `CARGO_TARGET_DIR` and `JIYUE_PYTHON` are honoured; `-NoZip` and `-OutDir` work; a bad `-Python` and a missing binary stop with a clear message.
- ARM64 (2026-11-06, development PC, x64 Windows 11): the arm64 build of the binaries did not run. The
  ARM64 MSVC tools are not installed (no `lib\arm64` in VC\Tools\MSVC), and no clang is on PATH; the build
  stops in the `ring` build script. The ARM64 WiX side was checked with placeholder binaries (x64 exes
  copied to `aarch64-pc-windows-msvc\release`): `wix build -arch arm64` succeeds, the MSI template is
  `Arm64;2052` (zh-CN), `wix msi validate` reports no errors, and the zip is created. An ARM64 MSI cannot be
  installed on this x64 PC; the install test has to run on an ARM64 Windows 11 PC.
- Not tested: upgrade from an earlier version (MajorUpgrade), repair, the GUI dialogs themselves (the silent tests do not show them), signing with a real certificate, and the first run on GitHub's `windows-latest` runner.

## Decisions and open points

- Why WiX 6 here and not cargo-packager (checked on 2026-11-06 with cargo-packager 0.11.8, `-f wix` and
  `-f nsis`, and the MSI tables read back with the Windows Installer COM object):
  - cargo-packager's WiX backend (WiX 3.11, downloaded) hard-codes `InstallScope="perMachine"`, Program
    Files, `ALLUSERS=1`, always adds a desktop shortcut, has no optional features and no zh-CN dialog texts
    (the dialogs stayed English), and gives no "Open with" entry. The only escape is a full custom template.
  - cargo-packager's NSIS backend does install per user (`installMode = "currentUser"`, `SimpChinese`
    UI, Start Menu and desktop shortcuts, silent `/S /D=` install and uninstall left no files behind). But it
    always creates a desktop shortcut, has no PATH option, and its `file-associations` made Jiyue the
    default handler for `.md` (it backed up the old default in `Markdown_backup` and did not restore it on
    uninstall). Not usable for this project.
  - cargo-dist 0.34.0 generates a per-machine WiX 3 MSI per package (`crates/*/wix/main.wxs`,
    `InstallScope='perMachine'`), one MSI for `jiyue` and one for `jiyue-mcp`, and no macOS bundles or Linux
    packages.
  - So the Windows MSI stays on this WiX 6 package; cargo-packager covers macOS and Linux.

- Manufacturer is `limitcool` (the repository owner). The product name stays `Jiyue 霁月`.
- Licence: `MIT OR Apache-2.0`. `LICENSE-MIT` and `LICENSE-APACHE` are in the repository root and are installed next to the program, together with `THIRD_PARTY_NOTICES.md`. The installer has no licence page; the files are installed and listed in the portable zip.
- Code signing: no certificate yet. The build has an optional signing step (see below). Unsigned installers trigger SmartScreen warnings.

## Licence files and third-party notices

- `LICENSE-MIT` and `LICENSE-APACHE` (repository root): the two licence texts of Jiyue.
- `THIRD_PARTY_NOTICES.md` (repository root): every crate linked into `jiyue.exe` and `jiyue-mcp.exe`, with its licence and repository, plus the bundled assets. It is regenerated by `build.ps1` with `scripts\gen-notices.py` (Python 3, standard library only, reads `cargo metadata`). The script exits with an error if a dependency has an unknown licence or only copyleft options.

## Code signing

`build.ps1` signs only when one of these environment variables is set; otherwise it prints `unsigned build (set JIYUE_SIGN_PFX to sign)` and continues.

| Variable | Meaning |
| --- | --- |
| `JIYUE_SIGN_PFX` | Path to a `.pfx` file (the certificate and its private key). |
| `JIYUE_SIGN_PFX_PASSWORD` | Password of the `.pfx`, if it has one. It is passed to `signtool` and never printed. |
| `JIYUE_SIGN_THUMBPRINT` | Thumbprint of a certificate in the current user's certificate store (instead of a `.pfx`). |
| `JIYUE_SIGN_TIMESTAMP` | Timestamp server. Default `http://timestamp.digicert.com`. |

When signing is on, the script copies `jiyue.exe` and `jiyue-mcp.exe` to `installer\out\signed`, signs the copies with `signtool sign /fd SHA256 /tr <timestamp> /td SHA256`, and checks them with `signtool verify /pa`. Only then is the MSI built, so it contains the signed binaries. The MSI is signed and verified last. `signtool.exe` is searched under `Windows Kits\10\bin\<version>\x64`; if it is missing, the build stops with a message. Install the Windows SDK to get it. The GitHub runner image is expected to include it; this is not verified yet.

```powershell
$env:JIYUE_SIGN_PFX = 'C:\path\to\jiyue.pfx'
$env:JIYUE_SIGN_PFX_PASSWORD = Read-Host -MaskInput 'PFX password'   # or set it in your own secret store
.\build.ps1
```

Do not use a self-signed certificate: it gives no SmartScreen benefit.

### What to obtain

The project has no certificate. Options, one line each:

- **OV code-signing certificate** (a `.pfx` or a hardware token): cheaper; SmartScreen reputation builds over time, so early downloads still show warnings.
- **EV code-signing certificate** (hardware token): more expensive; removes SmartScreen warnings immediately.
- **Azure Trusted Signing**: cheap, cloud-based signing; needs identity validation of the publisher before it can sign. Use it through `signtool` with its dlib, which is outside this script (add the call in `build.ps1` if chosen).
- **SignPath** (free for open-source projects, on approval): signing as a service; also outside this script.
- The MSI language is zh-CN only. An English string file can be added as a second `.wxl` if needed.
- The app's "open this file" support (for the `MarkdownVerb` feature) is an app change, not an installer change.
- The UpgradeCode is `{F48B69CA-0C87-4698-A62A-0D80DE358AAF}`. Never change it: it identifies the product for upgrades.
