[CmdletBinding()]
param(
    [ValidateSet("x64", "arm64")]
    [string]$Architecture = $(if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }),
    [string]$Version = ((Select-String '^version\s*=\s*"(.+)"' (Join-Path $PSScriptRoot "../../Cargo.toml")).Matches[0].Groups[1].Value),
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = Resolve-Path (Join-Path $PSScriptRoot "../..")

if (-not $SkipBuild) {
    Push-Location $repo
    try { cargo build --release --locked; if ($LASTEXITCODE) { throw "cargo build failed" } } finally { Pop-Location }
}

$binary = Join-Path $repo "target/release/nodestorm.exe"
if (-not (Test-Path $binary)) { throw "Missing $binary. Build first (drop -SkipBuild)." }

# Highest-versioned SDK bin holding an x64 makeappx.exe (host tool packs any target arch).
$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if (-not $makeappx) { throw "makeappx.exe not found. Install the Windows 10/11 SDK." }

$layout = Join-Path $env:TEMP "nodestorm-msix-layout-$Architecture"
& (Join-Path $PSScriptRoot "prepare-layout.ps1") -Binary $binary -Layout $layout -Architecture $Architecture

$out = Join-Path $repo "nodestorm-v$Version-windows-$Architecture.msix"
& $makeappx pack /o /d $layout /p $out
if ($LASTEXITCODE) { throw "makeappx pack failed" }
Write-Host "Built $out"
