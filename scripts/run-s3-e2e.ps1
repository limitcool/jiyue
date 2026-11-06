# Runs the S3 end-to-end sync tests (crates/jiyue-sync/tests/s3_e2e.rs) against a versioned test
# bucket. The bucket and its credentials come from a runner script that is kept outside the repo
# (it creates the bucket, passes JIYUE_TEST_S3_* as environment variables and deletes the bucket
# afterwards). Nothing secret is written here.
#
# Usage (PowerShell 7, from anywhere):
#   pwsh dev\sync-e2e\run-s3-e2e.ps1 -Runner <path to the runner script> [-Python <path to python.exe>]
#
# Extra arguments are passed to the test binary, e.g. -TestFilter version_history.

param(
    [Parameter(Mandatory = $true)] [string] $Runner,
    [string] $Python = $(if ($env:JIYUE_PYTHON) { $env:JIYUE_PYTHON } else { 'python' }),
    [string] $TestFilter = ''
)

$ErrorActionPreference = 'Stop'
$app = Resolve-Path (Join-Path $PSScriptRoot '..')
Push-Location $app
try {
    $cargoArgs = @('test', '-p', 'jiyue-sync', '--test', 's3_e2e', '--', '--ignored', '--test-threads=1', '--nocapture')
    if ($TestFilter) { $cargoArgs += $TestFilter }
    & $Python $Runner -- cargo @cargoArgs
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
