$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$setup = Join-Path $root "plugins\nodestorm\skills\nodestorm\scripts\setup.ps1"
$fixture = Join-Path ([System.IO.Path]::GetTempPath()) ("nodestorm-store-test-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory $fixture | Out-Null

try {
    $store = @{
        identityName = "Test.Nodestorm"
        publisher = "CN=Test"
        productId = "9TESTNODESTORM"
        executionAlias = "nodestorm.exe"
        msixVersion = "0.9.0.0"
        version = "0.9.0"
    } | ConvertTo-Json
    $storePath = Join-Path $fixture "store.json"
    Set-Content -Path $storePath -Value $store -Encoding utf8NoBOM

    $log = Join-Path $fixture "commands.log"
    $winget = Join-Path $fixture "winget.cmd"
    $curl = Join-Path $fixture "curl.cmd"
    Set-Content -Path $winget -Encoding ascii -Value "@echo off`r`necho winget %* >> `"$log`"`r`nexit /b 1`r`n"
    Set-Content -Path $curl -Encoding ascii -Value "@echo off`r`necho curl %* >> `"$log`"`r`nexit /b 22`r`n"

    $env:NODESTORM_SETUP_TESTING = "1"
    $env:NODESTORM_TEST_STORE_METADATA = $storePath
    $env:NODESTORM_TEST_WINGET = $winget
    $env:NODESTORM_TEST_CURL = $curl

    $failedAsExpected = $false
    try {
        & $setup -Architecture x64 -ApproveInstall -SkipLaunch
    } catch {
        if ($_.Exception.Message -match "Microsoft Store listing is unavailable") {
            $failedAsExpected = $true
        } else {
            throw
        }
    }
    if (-not $failedAsExpected) { throw "Store-not-live setup unexpectedly succeeded." }
    $commands = Get-Content $log -Raw
    if ($commands -notmatch "winget show") { throw "WinGet listing check did not execute." }
    if ($commands -notmatch "curl .*apps.microsoft.com/detail/9TESTNODESTORM") { throw "Store page check did not execute." }
    if ($commands -match "winget install") { throw "Installer ran after Store availability failed." }
    $global:LASTEXITCODE = 0
    Write-Host "Windows Store-not-live failure path passed."
} finally {
    Remove-Item Env:NODESTORM_SETUP_TESTING -ErrorAction SilentlyContinue
    Remove-Item Env:NODESTORM_TEST_STORE_METADATA -ErrorAction SilentlyContinue
    Remove-Item Env:NODESTORM_TEST_WINGET -ErrorAction SilentlyContinue
    Remove-Item Env:NODESTORM_TEST_CURL -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $fixture
}
