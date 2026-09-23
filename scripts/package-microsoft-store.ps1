param(
    [Parameter(Mandatory=$true)][string]$IdentityFile,
    [Parameter(Mandatory=$true)][string]$PortableDirectory,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [switch]$ValidationOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repoRoot = Split-Path $PSScriptRoot -Parent
$identity = Get-Content -LiteralPath $IdentityFile -Raw | ConvertFrom-Json
foreach ($property in @('identity_name','publisher','publisher_display_name','package_version')) {
    if (-not $identity.$property -or $identity.$property -match 'COPY_|__|PLACEHOLDER') { throw "Missing real Store identity field: $property" }
}
if ($identity.identity_name -notmatch '^[A-Za-z0-9.-]{3,50}$') { throw 'Invalid package identity name.' }
if ($identity.publisher -notmatch '^CN=') { throw 'Publisher must exactly match the Partner Center certificate subject.' }
if ($identity.package_version -notmatch '^[1-9][0-9]*\.[0-9]+\.[0-9]+\.0$') { throw 'Store package version must have a nonzero major and end in .0.' }
foreach ($part in $identity.package_version.Split('.')) { if ([uint64]$part -gt 65535) { throw 'Package version component exceeds 65535.' } }
if (-not $ValidationOnly -and ($identity.identity_name -match 'Test|Validation' -or $identity.publisher -match 'Test|Validation')) {
    throw 'Test identities cannot produce submission packages.'
}
$portable = (Resolve-Path -LiteralPath $PortableDirectory).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
$repoFull = [IO.Path]::GetFullPath($repoRoot).TrimEnd('\') + '\'
if ($output.StartsWith($repoFull, [StringComparison]::OrdinalIgnoreCase)) { throw 'Keep generated Store packages outside the source checkout.' }
if ($output.TrimEnd('\') -eq $portable.TrimEnd('\')) { throw 'Output directory must differ from portable input.' }
$exe = Join-Path $portable 'lawpdf.exe'
$distOut = Join-Path ([IO.Path]::GetTempPath()) ("lawpdf-store-status-" + [guid]::NewGuid().ToString() + '.json')
try {
    $process = Start-Process $exe -ArgumentList '--distribution-status' -Wait -PassThru -NoNewWindow -RedirectStandardOutput $distOut
    if ($process.ExitCode -ne 0) { throw 'Distribution status failed.' }
    $status = Get-Content -LiteralPath $distOut -Raw | ConvertFrom-Json
} finally { Remove-Item -LiteralPath $distOut -Force -ErrorAction SilentlyContinue }
if ($status.channel -ne 'microsoft-store' -or $status.self_updates_enabled -ne $false) { throw 'Build with --features microsoft-store; direct updater is not allowed.' }
New-Item -ItemType Directory -Force -Path $output | Out-Null
$stage = Join-Path $output ('stage-' + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -Path (Join-Path $portable '*') -Destination $stage -Recurse
$assetDir = Join-Path $stage 'Assets'
New-Item -ItemType Directory -Path $assetDir | Out-Null
Add-Type -AssemblyName System.Drawing
$source = [Drawing.Image]::FromFile((Join-Path $repoRoot 'assets\lawpdf.png'))
try {
    foreach ($spec in @(@('StoreLogo',50),@('Square44x44Logo',44),@('Square150x150Logo',150))) {
        $size = [int]$spec[1]
        $bitmap = [Drawing.Bitmap]::new($size,$size)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.Clear([Drawing.Color]::Transparent)
            $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.DrawImage($source,0,0,$size,$size)
            $bitmap.Save((Join-Path $assetDir ($spec[0]+'.png')),[Drawing.Imaging.ImageFormat]::Png)
        } finally { $graphics.Dispose(); $bitmap.Dispose() }
    }
} finally { $source.Dispose() }
$xml = Get-Content -LiteralPath (Join-Path $repoRoot 'packaging\microsoft-store\AppxManifest.xml.in') -Raw
$replacements = @{
    '__IDENTITY_NAME__'=$identity.identity_name
    '__PUBLISHER__'=$identity.publisher
    '__PUBLISHER_DISPLAY_NAME__'=$identity.publisher_display_name
    '__PACKAGE_VERSION__'=$identity.package_version
}
foreach ($key in $replacements.Keys) { $xml=$xml.Replace($key,[Security.SecurityElement]::Escape($replacements[$key])) }
[xml]$parsed = $xml
[IO.File]::WriteAllText((Join-Path $stage 'AppxManifest.xml'),$xml,[Text.UTF8Encoding]::new($false))
$kitRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$makeappx = Get-ChildItem -LiteralPath $kitRoot -Directory | Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\makeappx.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $makeappx) { throw 'Install the Windows SDK to obtain MakeAppx.' }
$label = if ($ValidationOnly) { 'validation-only' } else { 'store-submission' }
$msix = Join-Path $output ("LawPDF-$($identity.package_version)-x64-$label.msix")
& $makeappx pack /d $stage /p $msix /o
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx package validation failed.' }
$unpacked = Join-Path $output ('verify-' + [guid]::NewGuid().ToString())
& $makeappx unpack /p $msix /d $unpacked /o
if ($LASTEXITCODE -ne 0) { throw 'MSIX could not be unpacked for verification.' }
[xml]$actual = Get-Content -LiteralPath (Join-Path $unpacked 'AppxManifest.xml') -Raw
if ($actual.Package.Identity.Name -ne $identity.identity_name -or $actual.Package.Identity.Publisher -ne $identity.publisher -or $actual.Package.Identity.Version -ne $identity.package_version) { throw 'Packaged identity differs from approved identity.' }
$exeHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ((Get-FileHash -LiteralPath (Join-Path $unpacked 'lawpdf.exe') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $exeHash) { throw 'Packaged executable changed.' }
$evidence = [ordered]@{
    schema='lawpdf-msix-package-v1'; purpose=$label; app_version=$status.version;
    package_version=$identity.package_version; identity_name=$identity.identity_name;
    publisher=$identity.publisher; architecture='x64'; executable_sha256=$exeHash;
    package_sha256=(Get-FileHash -LiteralPath $msix -Algorithm SHA256).Hash.ToLowerInvariant();
    self_updates_enabled=$false; signed_by_microsoft=$false; store_approved=$false;
    package_path=$msix
}
$evidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $output 'package-evidence.json') -Encoding utf8
Write-Host "MSIX prepared and unpack-verified: $msix"
Write-Host 'Microsoft Store certification, signing and fresh Store installation remain required.'
