param(
    [Parameter(Mandatory=$true)][string]$Publisher,
    [Parameter(Mandatory=$true)][string]$PublisherDisplayName,
    [Parameter(Mandatory=$true)][string]$MakeAppx,
    [switch]$Signed
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$version = (Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw | ConvertFrom-Json).version
$staging = Join-Path $repo 'out/identity'
New-Item -ItemType Directory -Path (Join-Path $staging 'Assets') -Force | Out-Null
$manifest = Get-Content -LiteralPath (Join-Path $repo 'installer/sparse/AppxManifest.xml.in') -Raw
$manifest = $manifest.Replace('@PUBLISHER@',[Security.SecurityElement]::Escape($Publisher)).Replace('@PUBLISHER_DISPLAY@',[Security.SecurityElement]::Escape($PublisherDisplayName)).Replace('@VERSION@',$version)
[IO.File]::WriteAllText((Join-Path $staging 'AppxManifest.xml'),$manifest,[Text.UTF8Encoding]::new($false))
foreach ($name in @('StoreLogo.png','Logo150.png','Logo44.png')) { Copy-Item -LiteralPath (Join-Path $repo "assets/identity/$name") -Destination (Join-Path $staging 'Assets') -Force }
$output = Join-Path $repo "out/OmniSync-Identity-$version-x64.msix"
& $MakeAppx pack /o /nv /d $staging /p $output
if ($LASTEXITCODE -ne 0) { throw 'Identity package build failed' }
if ($Signed) {
    if (-not $env:OMNISYNC_SIGNTOOL -or -not $env:OMNISYNC_CERT_THUMBPRINT -or -not $env:OMNISYNC_TIMESTAMP_URL) { throw 'Signing configuration is incomplete' }
    & $env:OMNISYNC_SIGNTOOL sign /sha1 $env:OMNISYNC_CERT_THUMBPRINT /fd SHA256 /tr $env:OMNISYNC_TIMESTAMP_URL /td SHA256 $output
    if ($LASTEXITCODE -ne 0) { throw 'Identity package signing failed' }
}
Write-Output "Built $output. No package was registered."
