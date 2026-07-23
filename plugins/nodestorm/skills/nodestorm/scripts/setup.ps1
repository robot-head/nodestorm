[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$ApproveInstall,
    [switch]$ApproveLaunch,
    [switch]$SkipLaunch,
    [ValidateSet("x64", "arm64")]
    [string]$Architecture
)

$ErrorActionPreference = "Stop"
$Version = "1.0.0"
$McpUrl = "http://127.0.0.1:4747/mcp"
$testing = $env:NODESTORM_SETUP_TESTING -eq "1"
$storePath = Join-Path $PSScriptRoot "store.json"
if ($testing -and $env:NODESTORM_TEST_STORE_METADATA) { $storePath = $env:NODESTORM_TEST_STORE_METADATA }
$Store = Get-Content $storePath -Raw | ConvertFrom-Json

if ($ApproveLaunch -and $SkipLaunch) { throw "Choose either -ApproveLaunch or -SkipLaunch." }

if (-not $Architecture) {
    $Architecture = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
}

Write-Host "Nodestorm setup target: windows/$Architecture"
Write-Host "Microsoft Store Product ID: $($Store.productId)"
if ($DryRun) { exit 0 }

foreach ($field in @("identityName", "publisher", "productId", "executionAlias", "msixVersion")) {
    if (-not $Store.$field -or $Store.$field -like "REPLACE_*") {
        throw "The Microsoft Store listing is not configured. Partner Center identity field '$field' must be reserved before setup can continue."
    }
}
if ($Store.version -ne $Version) {
    throw "Store metadata version $($Store.version) does not match plugin version $Version."
}
if ($Store.msixVersion -ne "1.0.0.0") {
    throw "Store MSIX version $($Store.msixVersion) does not match 1.0.0.0."
}

function Confirm-Action([string]$Prompt) {
    $answer = Read-Host "$Prompt [y/N]"
    return $answer -eq "y" -or $answer -eq "Y"
}

if (-not $ApproveInstall -and -not (Confirm-Action "Install Nodestorm v$Version from Microsoft Store?")) {
    throw "Installation cancelled."
}

$listener = Get-NetTCPConnection -State Listen -LocalPort 4747 -ErrorAction SilentlyContinue
if ($listener) { throw "Port 4747 is already in use; setup will not launch into a conflict." }

$winget = Get-Command winget.exe -ErrorAction SilentlyContinue
$curl = Get-Command curl.exe -ErrorAction SilentlyContinue
$wingetPath = if ($winget) { $winget.Source } else { $null }
$curlPath = if ($curl) { $curl.Source } else { $null }
if ($testing -and $env:NODESTORM_TEST_WINGET) { $wingetPath = $env:NODESTORM_TEST_WINGET }
if ($testing -and $env:NODESTORM_TEST_CURL) { $curlPath = $env:NODESTORM_TEST_CURL }
if (-not $curlPath) { throw "curl.exe is required for Store and MCP readiness checks." }
$storeAvailable = $false
if ($wingetPath) {
    & $wingetPath show --id $Store.productId --source msstore --exact --accept-source-agreements | Out-Null
    $storeAvailable = $LASTEXITCODE -eq 0
}

if (-not $storeAvailable) {
    & $curlPath --fail --silent --show-error --head "https://apps.microsoft.com/detail/$($Store.productId)" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "The Microsoft Store listing is unavailable." }
}

$installed = $false
if ($wingetPath -and $storeAvailable) {
    & $wingetPath install --id $Store.productId --source msstore --exact --accept-source-agreements --accept-package-agreements
    $installed = $LASTEXITCODE -eq 0
}

if (-not $installed) {
    $storeUri = "ms-windows-store://pdp/?ProductId=$($Store.productId)"
    Start-Process $storeUri
    Write-Host "Complete the trusted Store installation in the opened window."
}

$windowsApps = [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps")) + [System.IO.Path]::DirectorySeparatorChar
$aliasPath = Join-Path $windowsApps $Store.executionAlias
$storePackage = $null
for ($attempt = 0; $attempt -lt 120; $attempt++) {
    $storePackage = Get-AppxPackage -Name $Store.identityName -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -eq $Store.identityName -and
            $_.Publisher -eq $Store.publisher -and
            $_.Version.ToString() -eq $Store.msixVersion
        } |
        Select-Object -First 1
    if ($storePackage -and (Test-Path -LiteralPath $aliasPath)) { break }
    Start-Sleep -Seconds 5
}
if (-not $storePackage) { throw "The installed Store package identity, publisher, or MSIX version does not match the certified listing." }
if (-not (Test-Path -LiteralPath $aliasPath)) { throw "The Store execution alias did not become available." }

$reportedVersion = (& $aliasPath --version | Out-String).Trim()
if ($reportedVersion -ne "nodestorm $Version") {
    throw "Installed Nodestorm version does not match $Version."
}

Write-Host "Installed Microsoft Store Nodestorm v$Version without administrator privileges or PATH changes."
if ($SkipLaunch) {
    Write-Host "Installed; launch skipped."
    exit 0
}
if (-not $ApproveLaunch -and -not (Confirm-Action "Launch Nodestorm now?")) {
    Write-Host "Installed; launch skipped."
    exit 0
}

$listener = Get-NetTCPConnection -State Listen -LocalPort 4747 -ErrorAction SilentlyContinue
if ($listener) { throw "Port 4747 became unavailable before launch." }
Start-Process $aliasPath

$initialize = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"nodestorm-setup","version":"1.0.0"}}}'
for ($attempt = 0; $attempt -lt 60; $attempt++) {
    $response = & $curlPath --silent --show-error --max-time 2 `
        -H "Content-Type: application/json" `
        -H "Accept: application/json, text/event-stream" `
        --data $initialize $McpUrl 2>$null
    if ($response -match '"serverInfo"') {
        Write-Host "Nodestorm MCP is ready at $McpUrl"
        exit 0
    }
    Start-Sleep -Seconds 1
}

throw "Nodestorm launched but MCP readiness timed out after 60 seconds."
