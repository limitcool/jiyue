#Requires -Version 7.0
<#
.SYNOPSIS
  Builds the Windows release binaries, the per-user MSI and the portable zip of Jiyue.

.DESCRIPTION
  Runs unchanged on a developer PC and on GitHub's windows-latest runner (see installer\README.md).
  Every path is derived from the location of this script; no drive letter or user name is hard-coded.

  1. Third-party notices: scripts\gen-notices.py (Python 3, standard library only) regenerates
     THIRD_PARTY_NOTICES.md. The build stops on an unknown or copyleft-only licence.
  2. Version from the [package] table of crates\jiyue-app\Cargo.toml.
  3. cargo build --release --locked -p jiyue -p jiyue-mcp into the target folder: $env:CARGO_TARGET_DIR
     if it is set, else target-dist (git-ignored, separate from the normal target builds).
  4. Optional code signing (only when JIYUE_SIGN_PFX or JIYUE_SIGN_THUMBPRINT is set).
  5. WiX: the local tool from installer\.config\dotnet-tools.json, its extensions cached in
     installer\.wix (added when missing). Builds Jiyue-<version>-<arch>.msi and runs wix msi validate.
  6. Portable zip Jiyue-<version>-<arch>-portable.zip (skipped with -NoZip). <arch> is x64 or arm64 (-Arch).
  7. Summary: paths, sizes and SHA-256 of every output.

  Outputs go to installer\out, or to -OutDir. Stops with a non-zero exit code on the first error.
  Never prompts. Never prints a password.

  Requirements: PowerShell 7, cargo (with the toolchain from rust-toolchain.toml), the .NET SDK
  (8 or newer, for the local WiX tool), and Python 3 (see -Python). Nothing is installed globally.

.PARAMETER SkipBuild
  Do not run cargo. Package the binaries already in the target folder (its release subfolder).
  Alias: SkipCargo.

.PARAMETER OutDir
  Output folder. Default: installer\out. A relative path is relative to the current folder.

.PARAMETER Arch
  Target architecture: x64 (default, the x86-64 MSI and zip) or arm64 (aarch64-pc-windows-msvc; the
  ARM64 MSI and zip, Jiyue-<version>-arm64.msi and Jiyue-<version>-arm64-portable.zip). The arm64 build
  uses --target, so its binaries are in target\aarch64-pc-windows-msvc\release. On an x64 PC the arm64
  build needs the MSVC ARM64 build tools (Visual Studio component "MSVC ... ARM64 build tools").

.PARAMETER NoZip
  Build only the MSI; do not create the portable zip.

.PARAMETER Python
  Python 3 interpreter for gen-notices.py. Default: the JIYUE_PYTHON environment variable, then the
  first working one of `python`, `py -3`, `python3` on PATH.

.EXAMPLE
  pwsh -File installer\build.ps1

.EXAMPLE
  pwsh -File installer\build.ps1 -SkipBuild -OutDir C:\temp\jiyue-out
#>
[CmdletBinding()]
param(
    [Alias('SkipCargo')]
    [switch]$SkipBuild,
    [ValidateSet('x64', 'arm64')]
    [string]$Arch = 'x64',
    [string]$OutDir,
    [switch]$NoZip,
    [string]$Python
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Fail([string]$message) {
    [Console]::Error.WriteLine("build.ps1: $message")
    exit 1
}

function Write-Step([string]$text) {
    Write-Host ''
    Write-Host "==> $text" -ForegroundColor Cyan
}

function Invoke-Checked([string]$what, [scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { Fail "$what failed with exit code $LASTEXITCODE" }
}

# ---- Paths (all derived from this script's location) -----------------------------------------
$installer     = $PSScriptRoot
$repo          = Split-Path -Parent $installer
$callerDir     = (Get-Location).ProviderPath

$targetDir     = if ($env:CARGO_TARGET_DIR) {
    [IO.Path]::GetFullPath([IO.Path]::Combine($repo, $env:CARGO_TARGET_DIR))
} else {
    Join-Path $repo 'target-dist'
}
# arm64 is always built with --target, so its binaries sit in a triple folder (also on an ARM64 PC).
$rustTriple    = if ($Arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { $null }
$releaseDir    = if ($rustTriple) { Join-Path $targetDir "$rustTriple\release" } else { Join-Path $targetDir 'release' }
$wixArch       = $Arch
$outDir        = if ($OutDir) {
    [IO.Path]::GetFullPath([IO.Path]::Combine($callerDir, $OutDir))
} else {
    Join-Path $installer 'out'
}
$staging       = Join-Path $outDir 'staging'
$signedDir     = Join-Path $outDir 'signed'
$objDir        = Join-Path $outDir 'obj'
$iconFile      = Join-Path $repo 'crates\jiyue-app\assets\logo\jiyue.ico'
$cargoToml     = Join-Path $repo 'crates\jiyue-app\Cargo.toml'
$notesScript   = Join-Path $repo 'scripts\gen-notices.py'
$wxsFile       = Join-Path $installer 'Package.wxs'
$wxlFile       = Join-Path $installer 'zh-CN.wxl'
$portableReadme = Join-Path $installer 'portable-README.txt'
$toolManifest  = Join-Path $installer '.config\dotnet-tools.json'
$licenseMit    = Join-Path $repo 'LICENSE-MIT'
$licenseApache = Join-Path $repo 'LICENSE-APACHE'
$noticesFile   = Join-Path $repo 'THIRD_PARTY_NOTICES.md'

# ---- Tools -----------------------------------------------------------------------------------
function Test-Command([string]$name) {
    return [bool](Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue)
}

function Get-PythonVersion([string]$Exe, [string[]]$PreArgs = @()) {
    # Returns "3.x.y" when the command is a working Python 3, otherwise $null. (The Windows Store
    # alias python.exe fails here, so it is not picked.)
    if (-not (Test-Command $Exe)) { return $null }
    $version = & $Exe @PreArgs -c 'import sys; print(".".join(map(str, sys.version_info[:3])))' 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    $version = "$version".Trim()
    if ($version -match '^3\.\d+\.\d+$') { return $version }
    return $null
}

function Resolve-Python {
    # Returns a hashtable: Exe, PreArgs (for example '-3' for py), Version.
    $explicit = if ($Python) { $Python } elseif ($env:JIYUE_PYTHON) { $env:JIYUE_PYTHON } else { $null }
    if ($explicit) {
        $version = Get-PythonVersion $explicit
        if (-not $version) { Fail "not a working Python 3 interpreter: $explicit (from -Python or JIYUE_PYTHON)" }
        return @{ Exe = $explicit; PreArgs = @(); Version = $version }
    }
    $candidates = @(
        @{ Exe = 'python';  PreArgs = @() },
        @{ Exe = 'py';      PreArgs = @('-3') },
        @{ Exe = 'python3'; PreArgs = @() }
    )
    foreach ($c in $candidates) {
        $version = Get-PythonVersion $c.Exe $c.PreArgs
        if ($version) { return @{ Exe = $c.Exe; PreArgs = $c.PreArgs; Version = $version } }
    }
    Fail 'no Python 3 found. Install Python 3 (as python, py -3 or python3 on PATH), or pass -Python <path> or set JIYUE_PYTHON.'
}

foreach ($tool in @('cargo', 'dotnet')) {
    if (-not (Test-Command $tool)) { Fail "$tool not found on PATH" }
}
foreach ($f in @($notesScript, $cargoToml, $iconFile, $licenseMit, $licenseApache, $wxsFile, $wxlFile, $portableReadme, $toolManifest)) {
    if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { Fail "missing file: $f" }
}

# ---- 0. Third-party notices -------------------------------------------------------------------
Write-Step 'Third-party notices (scripts\gen-notices.py)'
$py = Resolve-Python
$pyExe = $py.Exe
$pyPreArgs = @($py.PreArgs)
Write-Host "Python: $pyExe $($pyPreArgs -join ' ') (version $($py.Version))"
Invoke-Checked 'gen-notices.py' { & $pyExe @pyPreArgs $notesScript }

# ---- 1. Version -------------------------------------------------------------------------------
Write-Step 'Version'
$inPackage = $false
$version = $null
foreach ($line in Get-Content -LiteralPath $cargoToml) {
    if ($line -match '^\s*\[') { $inPackage = ($line.Trim() -eq '[package]'); continue }
    if ($inPackage -and $line -match '^\s*version\s*=\s*"(\d+\.\d+\.\d+)"\s*$') {
        $version = $Matches[1]
        break
    }
}
if (-not $version) { Fail "no x.y.z version found in $cargoToml" }
Write-Host "Version: $version"

# ---- 2. Release build -------------------------------------------------------------------------
if ($SkipBuild) {
    Write-Step "Release build skipped (-SkipBuild), using $releaseDir"
} else {
    Write-Step "cargo build --release --locked (target dir: $targetDir, arch: $Arch)"
    $previousTargetDir = $env:CARGO_TARGET_DIR
    $env:CARGO_TARGET_DIR = $targetDir
    $targetArgs = if ($rustTriple) { @('--target', $rustTriple) } else { @() }
    if ($rustTriple -and (Test-Command 'rustup')) {
        $installedTargets = @(& rustup target list --installed)
        if ($installedTargets -notcontains $rustTriple) {
            Invoke-Checked 'rustup target add' { rustup target add $rustTriple }
        }
    }
    Push-Location $repo
    try {
        Invoke-Checked 'cargo build' { cargo build --release --locked -p jiyue -p jiyue-mcp @targetArgs }
    } finally {
        Pop-Location
        $env:CARGO_TARGET_DIR = $previousTargetDir
    }
}

$jiyueExe = Join-Path $releaseDir 'jiyue.exe'
$mcpExe   = Join-Path $releaseDir 'jiyue-mcp.exe'
foreach ($f in @($jiyueExe, $mcpExe, $noticesFile)) {
    if (-not (Test-Path -LiteralPath $f -PathType Leaf)) {
        $hint = if ($SkipBuild) { ' (run without -SkipBuild first)' } else { '' }
        Fail "missing file: $f$hint"
    }
}

# Output folder; the MSI and zip of this version are replaced, other versions are kept.
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$msi = Join-Path $outDir "Jiyue-$version-$Arch.msi"
$zip = Join-Path $outDir "Jiyue-$version-$Arch-portable.zip"
Remove-Item -LiteralPath $msi, $zip -ErrorAction SilentlyContinue

# ---- 3. Optional code signing -----------------------------------------------------------------
# Off unless JIYUE_SIGN_PFX (a .pfx file, password in JIYUE_SIGN_PFX_PASSWORD if it has one) or
# JIYUE_SIGN_THUMBPRINT (a certificate in the current user's store) is set. The password is passed
# to signtool and never printed.
$signingPfx   = $env:JIYUE_SIGN_PFX
$signingThumb = $env:JIYUE_SIGN_THUMBPRINT
$signing      = [bool]($signingPfx -or $signingThumb)
$timestamp    = if ($env:JIYUE_SIGN_TIMESTAMP) { $env:JIYUE_SIGN_TIMESTAMP } else { 'http://timestamp.digicert.com' }
$signTool     = $null

function Find-SignTool {
    $roots = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'),
        (Join-Path $env:ProgramFiles 'Windows Kits\10\bin')
    )
    $found = foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) { continue }
        Get-ChildItem -LiteralPath $root -Directory |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
            ForEach-Object {
                $exe = Join-Path $_.FullName 'x64\signtool.exe'
                if (Test-Path -LiteralPath $exe -PathType Leaf) { [pscustomobject]@{ Version = [version]$_.Name; Path = $exe } }
            }
    }
    $best = $found | Sort-Object Version -Descending | Select-Object -First 1
    if (-not $best) {
        Fail 'signtool.exe not found. Install the Windows SDK ("Signing Tools for Desktop Apps") or see installer\README.md.'
    }
    return $best.Path
}

function Invoke-SignFile([string]$file) {
    $signArgs = @('sign', '/fd', 'SHA256', '/tr', $timestamp, '/td', 'SHA256')
    if ($signingPfx) {
        if (-not (Test-Path -LiteralPath $signingPfx -PathType Leaf)) { Fail 'JIYUE_SIGN_PFX does not point to an existing file' }
        $signArgs += @('/f', $signingPfx)
        if ($env:JIYUE_SIGN_PFX_PASSWORD) { $signArgs += @('/p', $env:JIYUE_SIGN_PFX_PASSWORD) }
    } else {
        $signArgs += @('/sha1', $signingThumb)
    }
    $signArgs += $file
    & $signTool @signArgs | Out-Host
    if ($LASTEXITCODE -ne 0) { Fail "signtool sign failed for $(Split-Path -Leaf $file) with exit code $LASTEXITCODE" }
    & $signTool verify /pa /q $file
    if ($LASTEXITCODE -ne 0) { Fail "signtool verify /pa failed for $(Split-Path -Leaf $file)" }
    Write-Host "  signed and verified: $(Split-Path -Leaf $file)"
}

if ($signing) {
    Write-Step 'Code signing'
    $signTool = Find-SignTool
    Write-Host "Signing with: $signTool (timestamp $timestamp)"
    Remove-Item -LiteralPath $signedDir -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $signedDir | Out-Null
    Copy-Item -LiteralPath $jiyueExe, $mcpExe -Destination $signedDir
    $jiyueExe = Join-Path $signedDir 'jiyue.exe'
    $mcpExe   = Join-Path $signedDir 'jiyue-mcp.exe'
    Invoke-SignFile $jiyueExe
    Invoke-SignFile $mcpExe
} else {
    Write-Step 'Code signing'
    Write-Host 'unsigned build (set JIYUE_SIGN_PFX to sign)'
}

# ---- 4. MSI with the local WiX tool -----------------------------------------------------------
function Ensure-WixExtensions([string]$wixVersion) {
    # Extensions are cached per folder (installer\.wix, run from installer\). List them and add only
    # the missing ones, so a clean runner (empty cache) and a developer PC both end up with the set.
    $names = @('WixToolset.UI.wixext', 'WixToolset.Util.wixext')
    $listed = @(& dotnet tool run wix extension list 2>$null)
    $missing = @($names | Where-Object {
        -not @($listed -match "^\s*$([regex]::Escape($_))\s+$([regex]::Escape($wixVersion))\s*$").Count
    })
    if ($missing.Count -eq 0) {
        Write-Host "WiX extensions cached: $($names -join ', ') $wixVersion"
        return
    }
    $addArgs = @($missing | ForEach-Object { "$_/$wixVersion" })
    Write-Host "Adding WiX extensions: $($addArgs -join ', ')"
    Invoke-Checked 'wix extension add' { dotnet tool run wix extension add @addArgs }
}

Write-Step 'MSI (WiX)'
$wixVersion = (Get-Content -LiteralPath $toolManifest -Raw | ConvertFrom-Json).tools.wix.version
if (-not $wixVersion) { Fail "no wix version in $toolManifest" }
Push-Location $installer
try {
    Invoke-Checked 'dotnet tool restore' { dotnet tool restore }
    Ensure-WixExtensions $wixVersion
    Invoke-Checked 'wix build' {
        dotnet tool run wix build `
            -arch $wixArch `
            -culture zh-CN `
            -loc $wxlFile `
            -ext WixToolset.UI.wixext `
            -ext WixToolset.Util.wixext `
            -sw1076 `
            -d "ProductVersion=$version" `
            -d "JiyueExeSource=$jiyueExe" `
            -d "JiyueMcpExeSource=$mcpExe" `
            -d "IconFile=$iconFile" `
            -d "LicenseMitSource=$licenseMit" `
            -d "LicenseApacheSource=$licenseApache" `
            -d "NoticesSource=$noticesFile" `
            -intermediatefolder $objDir `
            -o $msi `
            $wxsFile
    }
} finally {
    Pop-Location
}
if (-not (Test-Path -LiteralPath $msi -PathType Leaf)) { Fail "wix did not produce $msi" }

if ($signing) {
    Write-Step 'Signing the MSI'
    Invoke-SignFile $msi
}

# ICE validation (Windows Installer rules). ICE91 (WIX1076) is suppressed on purpose: it only warns that
# per-user files would not be copied per machine, and this package is per-user only.
Write-Step 'MSI validation (wix msi validate)'
Push-Location $installer
try {
    Invoke-Checked 'wix msi validate' { dotnet tool run wix msi validate -sice ICE91 $msi }
} finally {
    Pop-Location
}

# ---- 5. Portable zip --------------------------------------------------------------------------
if ($NoZip) {
    Write-Step 'Portable zip skipped (-NoZip)'
} else {
    Write-Step 'Portable zip'
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $staging | Out-Null
    try {
        Copy-Item -LiteralPath $jiyueExe, $mcpExe -Destination $staging
        Copy-Item -LiteralPath $portableReadme -Destination (Join-Path $staging 'README.txt')
        Copy-Item -LiteralPath $licenseMit, $licenseApache, $noticesFile -Destination $staging
        Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip -CompressionLevel Optimal
    } finally {
        Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
    }
    if (-not (Test-Path -LiteralPath $zip -PathType Leaf)) { Fail "could not create $zip" }
}

# ---- 6. Summary -------------------------------------------------------------------------------
Write-Step 'Summary'
Write-Host "Version      : $version"
Write-Host "Signing      : $(if ($signing) { 'yes (jiyue.exe, jiyue-mcp.exe and the MSI verified)' } else { 'no (set JIYUE_SIGN_PFX or JIYUE_SIGN_THUMBPRINT)' })"
Write-Host "Binaries     : $releaseDir (-SkipBuild: $SkipBuild)"
Write-Host "Target dir   : $targetDir"
$outputs = if ($NoZip) { @($msi) } else { @($msi, $zip) }
foreach ($f in $outputs) {
    $item = Get-Item -LiteralPath $f
    $hash = (Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash.ToLowerInvariant()
    $mb   = [math]::Round($item.Length / 1MB, 2)
    Write-Host ''
    Write-Host $item.FullName
    Write-Host ("  size   : {0} bytes ({1} MB)" -f $item.Length, $mb)
    Write-Host ("  sha256 : {0}" -f $hash)
}
Write-Host ''
Write-Host 'Done.'
exit 0
