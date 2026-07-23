[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("x64", "arm64")]
    [string]$Architecture,
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [string]$IdentityPath = (Join-Path $PSScriptRoot "store-identity.json")
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path $IdentityPath)) {
    throw "Missing $IdentityPath. Reserve Nodestorm in Partner Center and commit its exact public identity values."
}
$identity = Get-Content $IdentityPath -Raw | ConvertFrom-Json
foreach ($field in @("identityName", "publisher", "publisherDisplayName", "productId", "applicationId", "executionAlias", "msixVersion")) {
    $value = $identity.$field
    if (-not $value -or $value -like "REPLACE_*") { throw "Store identity field '$field' is not configured." }
}
if ($identity.msixVersion -ne "1.0.0.0") { throw "MSIX version must be 1.0.0.0." }

$manifest = Get-Content (Join-Path $PSScriptRoot "AppxManifest.template.xml") -Raw
$manifest = $manifest.Replace("@@IDENTITY_NAME@@", $identity.identityName)
$manifest = $manifest.Replace("@@PUBLISHER@@", $identity.publisher)
$manifest = $manifest.Replace("@@PUBLISHER_DISPLAY_NAME@@", $identity.publisherDisplayName)
$manifest = $manifest.Replace("@@VERSION@@", $identity.msixVersion)
$manifest = $manifest.Replace("@@ARCHITECTURE@@", $Architecture)
$manifest = $manifest.Replace("@@APPLICATION_ID@@", $identity.applicationId)
$manifest = $manifest.Replace("@@EXECUTION_ALIAS@@", $identity.executionAlias)

$parent = Split-Path $OutputPath -Parent
New-Item -ItemType Directory -Force $parent | Out-Null
[System.IO.File]::WriteAllText($OutputPath, $manifest, [System.Text.UTF8Encoding]::new($false))
