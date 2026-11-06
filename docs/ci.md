# Continuous integration and releases

Jiyue is built and tested by two workflow files in `.github/workflows/`. They use GitHub Actions syntax,
so they also run on Gitea Actions (for example a self-hosted Gitea). The Cargo workspace is at the repository root.
A release is **one workflow run**: `release.yml` calls `ci.yml` as its first job, so the tests and the
packages of a tag come from the same run (see "One run per release").

| Workflow | When | What |
| --- | --- | --- |
| `ci.yml` | pull requests, by hand, and called by `release.yml` (job `checks`) | rustfmt (Linux); build and test on Windows x64, macOS arm64, macOS x64 and Linux x64 (plus the ARM64 pair on public repositories, see below); clippy (blocking) |
| `release.yml` | tags `v*`, by hand | job `checks` (= `ci.yml`); packages: Windows MSI for x64 and ARM64 (`installer/`, WiX 6), macOS `.dmg` with `Jiyue.app` for arm64 and x64, Linux `.deb` and AppImage for x86_64 and aarch64 (cargo-packager 0.11.8); job `publish`: `SHA256SUMS` and the GitHub Release, only when the checks passed |

## One run per release

```
git tag v0.1.0 && git push origin v0.1.0
   |
   v
Release run (one workflow run, one page in the Actions tab)
   prepare ............ reads the version, checks it against the tag, builds the package matrices
     |
     +-- checks ........ = ci.yml: rustfmt, clippy, build and test on every OS
     |                    (jobs shown as "checks / <job name>")
     +-- windows ........ MSI x64 (+ arm64*)           \ run at the same time as checks,
     +-- macos .......... dmg arm64, dmg x64           / not after it
     +-- linux .......... deb + AppImage x86_64 (+ aarch64*)
                              |
     publish <----------------+  needs: prepare, checks, windows, macos, linux
                                 runs only if checks succeeded (never on a failed or cancelled check)
                                 creates the GitHub Release with every package that was built

   * ARM64 entries: only on a public repository, or the include_arm / ENABLE_ARM_RUNNERS rule (see below).
```

Rules of the flow:

- **A failed check blocks the release.** If any job of `checks` fails, `publish` is skipped and no GitHub
  Release is created. The packages are still built and kept as workflow artifacts (for the retention
  period of the repository, 90 days by default), so a fixed run can publish them.
- **A failed package does not block the others.** As before, `publish` still runs when some packages
  succeeded; the release is then marked as a pre-release and its notes list the missing platforms.
- **Tag runs skip the tests.** A tag push runs `checks` with format, clippy and the plan only; the test
  matrix runs on pull requests and on manual runs of `ci.yml`. Run the tests locally before tagging.
- **Pushes to `main` run nothing.** `ci.yml` has no push trigger, so the test matrix starts only on pull
  requests and manual runs. Commits on `main` are not tested on GitHub, and a manual run of `ci.yml` (Actions tab) checks any branch on demand. If a tag run fails
  before publishing, nothing is released: fix the cause with a new commit, then delete the unpublished tag and
  tag the new commit again, or release the next version number.
- **Duration.** The tag run takes as long as the longer of the tests and the packages, and `publish` starts
  after the tests. The slowest test job was 25 minutes on Windows in the last green run (see "CI speed").
- **Manual runs:** Actions, Release, Run workflow. Inputs: `tag` (required; a draft release is created),
  `include_arm` (the ARM64 entries on a private repository, one run), and `skip_checks` (default `false`).
  Tick `skip_checks` only to publish when the tests are known to be irrelevant to the change, for example
  to rebuild packages after a packaging-only fix. The release notes then say "Tests were skipped". A tag
  push cannot skip the tests.

Gotchas that the design handles (see the comments in both files):

- **Concurrency names.** Inside a called workflow `github.workflow` is the caller's name, so `ci.yml`
  uses its own group, `ci-<event>-<ref>`, and `release.yml` uses `release-<ref>`. A shared name would make
  the callee cancel the caller.
- **Cancellation.** `ci.yml` cancels older runs only for pull requests and branches other than `main`
  (`cancel-in-progress: ${{ github.ref_type != 'tag' && github.ref != 'refs/heads/main' }}`). Tag runs and
  pushes to `main` are never cancelled by a later push. A tag run is never cancelled at all: `release.yml`
  does not cancel in progress.
- **Permissions.** A called workflow cannot ask for more than its caller. Both use `contents: read`; only
  `publish` has `contents: write`.
- **Context.** In a called workflow `github.event_name`, `github.ref`, `github.ref_type` and
  `github.event.repository.private` are the caller's values (a tag push, for example). `inputs.include_arm`
  is passed from `release.yml` (`false` for a tag push). No secrets are passed: `ci.yml` needs none.
- **Job names.** The checks are shown as `checks / rustfmt`, `checks / clippy`, `checks / build and test (windows-x64)`,
  and so on. The pull request and `main` runs of `ci.yml` keep the plain names. A branch protection rule that
  names a CI job must use the name of the run it is meant for.

Known risk: the `checks` job calls `ci.yml` with `uses: ./.github/workflows/ci.yml`. Reusable workflows are
a GitHub Actions feature. On a Gitea Actions host, check that the Gitea version supports a local reusable
workflow; if not, `checks` fails and `publish` never runs. Then copy the jobs into `release.yml` instead.

## Versions

- Rust is pinned to **1.99.1** in three places that must match: `rust-toolchain.toml`,
  `RUST_VERSION` in both workflows. Edition 2024 needs Rust 1.85 or newer.
- `--locked` is used everywhere: a `Cargo.lock` that is out of date fails the job.
- Actions used: `actions/checkout@v7`, `actions/upload-artifact@v7`, `actions/download-artifact@v8`,
  `actions/setup-python@v7`, `actions/setup-dotnet@v6`, `Swatinem/rust-cache@v2`,
  `softprops/action-gh-release@v2`, and `dtolnay/rust-toolchain@master` (the action has no major tags;
  the toolchain is set by the `toolchain` input).
- cargo-packager is pinned in `release.yml` (`CARGO_PACKAGER_VERSION`, currently `0.11.8`) and installed with
  `cargo install --locked`, because upstream publishes no prebuilt binaries. Its settings live in
  `[package.metadata.packager]` in `crates/jiyue-app/Cargo.toml`.
- Cache keys: `Swatinem/rust-cache` keys on the runner OS and the `Cargo.lock` hash. In `ci.yml` the test
  jobs share one key per matrix entry (`shared-key: test-<name>`) and only `main` saves the cache
  (`save-if`); pull requests and tag runs restore it. In `release.yml` each package job has its own key per
  architecture (`windows-release-<arch>`, `macos-release-<arch>`, `linux-release-<arch>`).
- Runner labels (checked against actions/runner-images, 2026-11-06): `windows-latest`, `windows-11-arm`
  (Windows 11 Arm64 with Visual Studio 2026, which includes the MSVC ARM64 tools and LLVM), `macos-latest`
  (Apple silicon), `macos-15-intel` (macOS 15, x86_64), `ubuntu-24.04`, `ubuntu-24.04-arm`. `macos-13` is no
  longer offered, so `macos-15-intel` is the Intel label used here.
- `rust-toolchain.toml` sets only the channel. It does not set `targets`, so every macOS job builds for its
  own host (`x86_64-apple-darwin` on `macos-15-intel`, `aarch64-apple-darwin` on `macos-latest`). No
  cross-compilation is involved.

## Required secrets (all optional)

Nothing is required for CI. The release jobs sign only when the secrets are set.

| Secret | Used by | Effect when missing |
| --- | --- | --- |
| `JIYUE_SIGN_PFX_BASE64`, `JIYUE_SIGN_PFX_PASSWORD` | Windows release | unsigned MSI and exes (SmartScreen warnings) |
| `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD` | macOS release, both arches (certificate import by cargo-packager, as `APPLE_CERTIFICATE`) | unsigned app |
| `JIYUE_MACOS_SIGN_IDENTITY` | macOS release, both arches: written into `signing-identity` in `Cargo.toml` by a CI step | not signed |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | macOS release (notarization, `APPLE_PASSWORD`), both arches | not notarized (Gatekeeper warning) |

Signing and notarization are both done only when the certificate and the app password are set; the
two macOS packaging steps differ only in their environment. The signing identity is not an environment
variable in cargo-packager 0.11.8, which is why the CI step writes it into `crates/jiyue-app/Cargo.toml`
(the placeholder line `# signing-identity = ...` under `[package.metadata.packager.macos]`). The step runs in
each macOS job separately, so it changes only the checkout of that job.

The Windows job decodes `JIYUE_SIGN_PFX_BASE64` into a temporary `.pfx`, passes its path to
`installer/build.ps1` as `JIYUE_SIGN_PFX`, and deletes it at the end. `build.ps1` is used unchanged.
The rest of the Windows job needs nothing from the runner beyond what the workflow installs:
`actions/setup-python@v7` (Python 3.12, its path is passed to `build.ps1` as `JIYUE_PYTHON` for
`scripts/gen-notices.py`, standard library only), `actions/setup-dotnet@v6` (.NET 8 and 9, for the
local WiX tool), `dotnet tool restore` in `installer/`, then `installer/build.ps1`. The script derives
every path from its own location, and the WiX extensions are added on first use. No drive letter,
user name or pre-made virtual environment is assumed.

## How to cut a release

1. Set `version` in `crates/jiyue-app/Cargo.toml` (the `[package]` table) and commit it to `main`
   (`ci.yml` tests that commit on the push).
2. `git tag v0.1.0 && git push origin v0.1.0`. That is the whole release: one Release run tests the
   tagged commit and publishes it (see "One run per release").

The `prepare` job fails when the tag does not equal `v` + that version. Without a tag (manual run),
`workflow_dispatch` takes a tag name and creates a **draft** release. Output files:

- Windows x64: `Jiyue-<version>-x64.msi` (per user, zh-CN)
- Windows ARM64 (only when the ARM64 entries are on, see "ARM64 runners"): `Jiyue-<version>-arm64.msi`
  (per user, zh-CN, `Arm64` platform). The job runs
  `installer/build.ps1 -Arch arm64 -NoZip` on `windows-11-arm`, which builds `aarch64-pc-windows-msvc` with `--target`.
- macOS arm64 (`macos-latest`): `Jiyue_<version>_aarch64.dmg`, a `.dmg` with `Jiyue.app` (`jiyue` and
  `jiyue-mcp` inside). macOS x64 (`macos-15-intel`): `Jiyue_<version>_x64.dmg`, the same content for Intel Macs.
  cargo-packager builds the dmg name from the product name (`Jiyue`), the version and the host architecture
  (`x86_64` is written as `x64`, `aarch64` stays `aarch64`); this was read in the cargo-packager 0.11.8
  source (`src/package/dmg/mod.rs`). The real names are checked on the first run.
- Linux x86_64: `jiyue_<version>_amd64.deb`, and `jiyue_<version>_amd64.AppImage` when the optional
  AppImage step succeeds
- Linux ARM64 (only when the ARM64 entries are on): `jiyue_<version>_arm64.deb`, and
  `jiyue_<version>_aarch64.AppImage` when the optional step succeeds. The job builds natively on `ubuntu-24.04-arm` (no `--target`, because cargo-packager reads
  `target/release`). The deb and AppImage names are cargo-packager's and were not checked on a run yet.
- `SHA256SUMS` (`sha256sum` over all files, added by the publish job)

Workflow artifacts of the packages are named `release-windows-<arch>`, `release-macos-<arch>` (`arm64` or
`x64`) and `release-linux-<arch>`. The publish job downloads every `release-*` artifact into one folder.

Only one place holds the GUI metadata: `[package.metadata.packager]` in `crates/jiyue-app/Cargo.toml`
(product name, bundle identifier, icons, licence and notice resources, the Markdown document type).
The Windows MSI keeps its own metadata in `installer/Package.wxs` and `installer/zh-CN.wxl`: the
cargo-packager WiX backend cannot do a per-user install, optional features or Simplified Chinese dialogs
(see `installer/README.md`), so it is not used for Windows.

## ARM64 runners and the visibility rule

The ARM64 hosted runners (`windows-11-arm`, `ubuntu-24.04-arm`) are free only for public repositories
and are not available to private repositories on the free plan: a job that asks for them in a private
repository waits in the queue forever. So the ARM64 entries are decided at run time:

- `ci.yml`, job `plan`, and `release.yml`, job `prepare`, read `github.event.repository.private`
  (`false` for a public repository; it is in the webhook payload of push, pull request, tag and
  workflow_dispatch events). The matrix is then built as JSON and read with `fromJSON`.
- The ARM64 entries are added when the repository is public, **or** when `include_arm` is set on a manual
  run (workflow_dispatch input, default `false`), **or** when the repository variable
  `ENABLE_ARM_RUNNERS` is `true`. If the repository value is missing (for example on a host that does not
  set it, such as Gitea), the ARM64 entries stay off. A tag run passes `include_arm` = `false` from
  `release.yml` to the `checks` job, so there the repository rule and the variable decide.
- The plan step prints the decision: `repository.private=..., ARM64 entries: ... (reason)`.

To enable them:

1. Public repository: make the repository public. Nothing else is needed.
2. Private repository, once the owner has ARM64 runners (larger runners with the same labels, or a
   plan that allows them): Settings, Secrets and variables, Actions, Variables, new variable
   `ENABLE_ARM_RUNNERS` = `true`. This enables them for every later run.
3. Private repository, one run only: Actions, choose Release (or CI), Run workflow, tick `include_arm`.

Do not enable them on a private repository without such runners: the queued job blocks the publish job
in `release.yml` until it is cancelled.

## Platform status

| Platform | Built and tested in CI | Notes |
| --- | --- | --- |
| Windows 11 x64 | yes (`windows-latest`) | MSI installer |
| macOS arm64 (Apple silicon) | yes (`macos-latest`) | unsigned unless secrets are set |
| macOS x86_64 (Intel) | yes (`macos-15-intel`) | unsigned unless secrets are set; dmg `Jiyue_<version>_x64.dmg`; see "Known gaps" |
| Linux x86_64 | yes (`ubuntu-24.04`) | X11 and Wayland through GPUI; title bar drawn by the app |
| Windows 11 ARM64 | yes (`windows-11-arm`), only when the ARM64 entries are on | MSI built by CI; the MSI installs only on ARM64 Windows |
| Linux ARM64 | yes (`ubuntu-24.04-arm`), only when the ARM64 entries are on | deb and AppImage built by CI |


Known differences: on Linux the window has client-side decorations (the tab strip is the title bar) and
gpui does not hit-test the drag and caption areas, so the app handles those clicks itself. On macOS the
native traffic lights stay at the top left and the app draws no caption buttons. Keyboard shortcuts use
Command instead of Ctrl on macOS (`ui/keymap.rs`, `platform_spelling`; `docs/shortcuts.md` still lists the Windows keys). Fonts are system fonts looked up
by name (Noto Sans SC, JetBrainsMono Nerd Font); none are bundled, so install them on Linux and macOS
for the intended look.

## CI speed

The test job of `ci.yml` was the slowest on Windows (last green run, job `build and test (windows-x64)`:
25 min, against 5 min on macOS and 6 min on Linux). Baseline steps: rust-cache restore 95 s, `cargo test`
1040 s, release build 211 s, rust-cache save 148 s. Changes made:

1. `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` (top-level `env` of `ci.yml`): no debug
   info in dev and test builds, so less PDB writing and linking on MSVC, and a smaller cache.
   `CARGO_INCREMENTAL=0` and `CARGO_NET_RETRY=5` are also set there.
2. The release build smoke test (`cargo build --release`) runs only on `linux-x64` and only for manual (`workflow_dispatch`) runs. `release.yml` builds
   the release binaries at tag time, so the Windows and macOS test jobs no longer spend 211 s on it.
3. `cargo test --workspace --locked --no-fail-fast`: every test binary runs even if one fails. The same
   tests run as before.
4. rust-cache: one shared key per matrix entry (`test-<name>`) and `save-if` only on `main`, so pull
   requests do not spend time saving the cache (148 s on Windows in the baseline).

Not done, on purpose:

- The Cargo target directory on the `D:` drive: `Swatinem/rust-cache` joins the `workspaces` target path to
  the root (`path.join`), so an absolute `D:\...` path cannot be cached. It would need a symlink.
  The `D:` drive is also not mentioned in the Windows runner image readme.
- `cargo nextest`: not adopted. It would add a tool and a second run for the doc tests, for a gain that
  was not measured.

Expected effect, to be measured on the runner and recorded here: the 211 s release build leaves the
Windows test job, the 148 s save leaves pull request runs, and the smaller debug-free cache restores
faster. The compile and test time itself (about 1040 s) is not reduced by these changes.

Adding the Intel macOS entry adds one more test job per run (the `macos-15-intel` runner is slower than
Apple silicon, and private repositories pay the macOS minute rate for it). It runs on every CI run and
every tag.

## Running the same checks locally

Windows (PowerShell), from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p jiyue -p jiyue-mcp --locked
installer\build.ps1 -SkipBuild                       # MSI and zip from the release binaries
installer\build.ps1 -Arch arm64                      # ARM64 MSI and zip (needs the MSVC ARM64 tools, see below)
```

The ARM64 build on an x64 PC needs the Visual Studio component for the MSVC ARM64 build tools and the
`aarch64-pc-windows-msvc` Rust target (`build.ps1` adds the target itself). The ARM64 binaries of the
development PC could not be built this way: the ARM64 MSVC tools and clang were not installed there.

macOS or Linux, from the repository root, after the release build (install the pinned tool first):

```bash
cargo install --locked --version 0.11.8 cargo-packager
cargo packager -p jiyue --release -f app,dmg     # macOS (arm64 or Intel, whichever the machine is)
cargo packager -p jiyue --release -f deb         # Linux
cargo packager -p jiyue --release -f appimage    # Linux, needs network (downloads linuxdeploy)
```

cargo-packager reads the binaries from `target/release` (`binaries-dir`) and writes to `dist/`
(`out-dir`), both relative to `crates/jiyue-app`. The `dist/` folder is therefore at the repository root.

## Linux system packages

The release and CI jobs install these with `apt-get` (Ubuntu 24.04):

```
pkg-config cmake clang libclang-dev libfontconfig-dev libfreetype-dev
libasound2-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev
libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libzstd-dev
libvulkan-dev mesa-vulkan-drivers
```

Why: `libfontconfig-dev` and
`libfreetype-dev` for font discovery (`font-kit`), `libclang-dev`/`clang` for `clang-sys`, `cmake` for
native builds, the X11, xkbcommon and Wayland packages for GPUI's windowing (most are loaded at run time).
OpenSSL is not needed: all TLS uses rustls.

## Clippy

`clippy` runs `cargo clippy --workspace --all-targets --locked -- -D warnings` on Linux and is blocking:
any warning fails the job and the merge. It was made a gate once the warning count reached zero. Keep it
that way: fix a new lint rather than adding a crate-level `allow`. An item-level `#[allow(clippy::...)]`
is acceptable only with a one-line reason comment on the item. Run the same command locally before
pushing; it must exit 0 on Windows and on the Linux target.

## Known gaps

- No macOS or Linux machine was available while this was written. The first runs have not happened.
  The cargo-packager steps (`.dmg`, `.app`, `.deb`, AppImage) are unverified. Its
  macOS `Info.plist` gets no `LSHandlerRank`, the bundle and desktop names are "Jiyue" (the Chinese name
  霁月 appears only in the Windows installer and the README), and the `.deb` dependencies are whatever
  cargo-packager detects. Also unverified: the macOS signing identity step (`sed` on `Cargo.toml`) and
  notarization with the app-specific password.
- macOS x86_64 (`macos-15-intel`) is built by CI and released as `Jiyue_<version>_x64.dmg`, but no run has
  happened yet. The `macos-15-intel` label is the Intel macOS image offered at the time of writing, and GitHub
  may retire it. When it is gone, remove the `macos-x64` entry from the matrix in the `plan` job of `ci.yml`
  and the x64 entry in the `macos` matrix of `release.yml` (the `macos_matrix` line of `prepare`), and update
  this file and the README. Until then, a job that cannot get a runner may stay queued, and `publish` waits
  for it: cancel that job by hand if it does.
- The AppImage download uses the unpinned `continuous` linuxdeploy release; the job is optional.
- ARM64, first runs: nothing has been built for ARM64 by CI yet. Crates that compile C or assembly
  (`ring`, `aws-lc-sys`, `zstd-sys`, `psm`, `clang-sys`) and the GPUI dependencies (`blade-graphics`,
  `font-kit`) are the likely failures. The windows-11-arm image has the MSVC ARM64 tools and LLVM, so
  `ilammy/msvc-dev-cmd` is not added; add it only if the first run shows a missing `cl` or `link` for ARM64.
- Gitea runners need labels that match `runs-on` (`windows-latest`, `macos-latest`, `macos-15-intel`,
  `ubuntu-24.04`). Change the labels in the workflows, or register runners with these names. Reusable
  workflows are needed for the `checks` job of `release.yml` (see "One run per release").
- The Windows release job has not run on GitHub yet. The local build of `installer/build.ps1` with a clean
  WiX extension cache and the `-SkipBuild` path are verified (see `installer/README.md`); the first
  `windows-latest` run is not. Its unknowns: the NuGet download of the WiX tool and extensions, and
  `signtool` if signing is turned on.
- The single-run release has not run on GitHub yet. Watch these on the first tag run:
  1. The run is named "Release" and shows `checks` with the nested jobs `checks / rustfmt`, `checks / clippy`,
     `checks / build and test (...)` for every OS, next to `windows`, `macos` (two entries), `linux`, `publish`.
  2. `checks` starts together with `prepare` and the packages, not after them. A "workflow file is invalid"
     error at the start of the run means an expression or input problem in `release.yml` or `ci.yml`.
  3. The `macos` jobs show `macOS app and dmg (arm64)` and `macOS app and dmg (x64)`, and the artifacts are
     `release-macos-arm64` and `release-macos-x64`. The dmg names are `Jiyue_<version>_aarch64.dmg` and
     `Jiyue_<version>_x64.dmg`; if the x64 name differs, the release asset list will show it.
  4. A deliberately failing check (for example a clippy warning on a test tag) leaves `publish` skipped and
     creates no Release.
  5. The `publish` job runs only after `checks` ends with success; the release is not a pre-release unless a
     package failed.
  6. The `macos x64` jobs get a runner (see "Known gaps"); a job stuck in the queue blocks `publish`.
- Clippy is clean on Windows and the Linux target (blocking, see above). rustfmt was applied once to the
  whole workspace when the CI was added, so the fmt job starts green; keep it that way with `cargo fmt --all`.
