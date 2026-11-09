# Builds the Windows installer of uniwidged from this checkout.
#
#   packaging\windows\build.ps1 [-Output DIR] [-TargetDir DIR]
#
# Needs `cargo` and the WiX 5 command-line tool (`dotnet tool install --global
# wix --version 5.*`), and says which is missing rather than failing halfway.
#
# Signing is optional and happens only when it is asked for: with
# `UNIWIDGED_SIGN_CERT` (a path to a .pfx) and `UNIWIDGED_SIGN_PASSWORD` set,
# the binary and then the installer are signed with `signtool`. Without them the
# installer is built unsigned and says so — SmartScreen will warn about it, and
# that is the truth about an unsigned installer rather than something to hide.
param(
    [string]$Output = "",
    [string]$TargetDir = ""
)
$ErrorActionPreference = "Stop"

$root = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Output) { $Output = Join-Path $root "target\windows" }
if (-not $TargetDir) { $TargetDir = Join-Path $root "target" }

foreach ($tool in @("cargo", "wix")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool is needed to build the installer and is not on this machine"
    }
}

# The version comes from the workspace manifest, as for the .deb.
$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.*)"$' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "no version in Cargo.toml" }

Write-Host "building uniwidged $version"
cargo build --release --locked --package uniwidged-cli --target-dir $TargetDir
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
$binary = Join-Path $TargetDir "release\uniwidged.exe"
if (-not (Test-Path $binary)) { throw "$binary was not built" }

function Sign($path) {
    if ($env:UNIWIDGED_SIGN_CERT -and $env:UNIWIDGED_SIGN_PASSWORD) {
        signtool sign /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com `
            /f $env:UNIWIDGED_SIGN_CERT /p $env:UNIWIDGED_SIGN_PASSWORD $path
        if ($LASTEXITCODE -ne 0) { throw "signing $path failed" }
        return $true
    }
    return $false
}

$signed = Sign $binary
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$msi = Join-Path $Output "uniwidged-$version-x64.msi"
wix build "$PSScriptRoot\uniwidged.wxs" -arch x64 `
    -d "Version=$version" -d "Binary=$binary" -o $msi
if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
$signed = (Sign $msi) -and $signed

if ($signed) {
    Write-Host "signed: $msi"
} else {
    Write-Host "built UNSIGNED: $msi (set UNIWIDGED_SIGN_CERT and UNIWIDGED_SIGN_PASSWORD to sign)"
}
