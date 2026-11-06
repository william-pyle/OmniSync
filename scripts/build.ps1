param([switch]$SkipTests,[switch]$Signed)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
$previousRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
try {
    $env:CARGO_ENCODED_RUSTFLAGS = @('-C','target-feature=+crt-static',"--remap-path-prefix=$repo=.","--remap-path-prefix=$env:USERPROFILE\.cargo=/.cargo") -join [char]31
    function Invoke-Checked([string]$Program, [string[]]$Arguments) {
        & $Program @Arguments
        if ($LASTEXITCODE -ne 0) { throw "$Program failed with exit code $LASTEXITCODE" }
    }
    Invoke-Checked 'npm.cmd' @('ci')
    Invoke-Checked 'npm.cmd' @('run','build')
    if (-not $SkipTests) { Invoke-Checked 'cargo.exe' @('test','--workspace','--locked') }
    Invoke-Checked 'cargo.exe' @('build','--workspace','--release','--locked','--features','omnisync/custom-protocol')
    $version = (Get-Content -LiteralPath 'package.json' -Raw | ConvertFrom-Json).version
    $payload = Join-Path $repo "out/payload/$version"
    New-Item -ItemType Directory -Path (Join-Path $payload 'icons') -Force | Out-Null
    foreach ($name in @('omnisync.exe','omnisync_shell.dll','omnisync-setup.exe')) {
        Copy-Item -LiteralPath (Join-Path $repo "target/release/$name") -Destination $payload -Force
    }
    Get-ChildItem -LiteralPath (Join-Path $repo 'assets/icons') -Filter '*.ico' | Copy-Item -Destination (Join-Path $payload 'icons') -Force
    Copy-Item -LiteralPath (Join-Path $repo 'README.md') -Destination $payload -Force
    Invoke-Checked (Join-Path $payload 'omnisync-setup.exe') @('verify-shell')
    if ($Signed) {
        if (-not $env:OMNISYNC_SIGNTOOL -or -not $env:OMNISYNC_CERT_THUMBPRINT -or -not $env:OMNISYNC_TIMESTAMP_URL) { throw 'Set all signing variables before using -Signed.' }
        foreach ($name in @('omnisync.exe','omnisync_shell.dll','omnisync-setup.exe')) {
            Invoke-Checked $env:OMNISYNC_SIGNTOOL @('sign','/sha1',$env:OMNISYNC_CERT_THUMBPRINT,'/fd','SHA256','/tr',$env:OMNISYNC_TIMESTAMP_URL,'/td','SHA256',(Join-Path $payload $name))
        }
    }
    $candidates = @(
        $(if ($env:NSIS_DIR) { Join-Path $env:NSIS_DIR 'makensis.exe' }),
        $(if ($env:ProgramFiles) { Join-Path ${env:ProgramFiles(x86)} 'NSIS/makensis.exe' }),
        (Join-Path $env:LOCALAPPDATA 'tauri/NSIS/makensis.exe'),
        (Join-Path $env:LOCALAPPDATA 'tauri/NSIS/Bin/makensis.exe')
    ) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }
    $compiler = $candidates | Select-Object -First 1
    if (-not $compiler) { throw 'NSIS 3 is required. Set NSIS_DIR to its directory.' }
    $installer = Join-Path $repo "out/OmniSync-Setup-$version-x64.exe"
    Invoke-Checked $compiler @("/DVERSION=$version","/DPAYLOAD=$payload","/DOUTPUT=$installer",(Join-Path $repo 'installer/OmniSync.nsi'))
    if ($Signed) { Invoke-Checked $env:OMNISYNC_SIGNTOOL @('sign','/sha1',$env:OMNISYNC_CERT_THUMBPRINT,'/fd','SHA256','/tr',$env:OMNISYNC_TIMESTAMP_URL,'/td','SHA256',$installer) }
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash.ToLowerInvariant()
    "$hash  $(Split-Path -Leaf $installer)" | Set-Content -LiteralPath "$installer.sha256" -Encoding ascii
    Write-Output "Built $installer"
} finally { $env:CARGO_ENCODED_RUSTFLAGS = $previousRustFlags; Pop-Location }
