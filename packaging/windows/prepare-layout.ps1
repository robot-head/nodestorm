[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary,
    [Parameter(Mandatory = $true)]
    [string]$Layout,
    [Parameter(Mandatory = $true)]
    [ValidateSet("x64", "arm64")]
    [string]$Architecture
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Layout | Out-Null
New-Item -ItemType Directory -Force (Join-Path $Layout "Assets") | Out-Null
Copy-Item $Binary (Join-Path $Layout "nodestorm.exe")
Copy-Item (Join-Path $PSScriptRoot "../../assets/nodestorm-mark.svg") (Join-Path $Layout "Assets/nodestorm-mark.svg")
& (Join-Path $PSScriptRoot "generate-manifest.ps1") -Architecture $Architecture -OutputPath (Join-Path $Layout "AppxManifest.xml")

Add-Type -AssemblyName System.Drawing
$source = [System.Drawing.Image]::FromFile((Resolve-Path (Join-Path $PSScriptRoot "../../docs/demo/poster.png")))
try {
    foreach ($asset in @(
        @{ Name = "StoreLogo.png"; Width = 50; Height = 50 },
        @{ Name = "Square44x44Logo.png"; Width = 44; Height = 44 },
        @{ Name = "Square150x150Logo.png"; Width = 150; Height = 150 },
        @{ Name = "Wide310x150Logo.png"; Width = 310; Height = 150 }
    )) {
        $bitmap = [System.Drawing.Bitmap]::new($asset.Width, $asset.Height)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try { $graphics.DrawImage($source, 0, 0, $asset.Width, $asset.Height) } finally { $graphics.Dispose() }
            $bitmap.Save((Join-Path $Layout "Assets/$($asset.Name)"), [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $bitmap.Dispose() }
    }
} finally { $source.Dispose() }
