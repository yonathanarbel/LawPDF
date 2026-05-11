param(
    [ValidateSet("debug", "release")]
    [string]$Configuration = "release"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$targetDir = Join-Path $repoRoot "target\$Configuration"
$exeName = "lawpdf.exe"
$exePath = Join-Path $targetDir $exeName

if (-not (Test-Path $exePath)) {
    throw "Missing $exePath. Run cargo build --$Configuration first."
}

$distDir = Join-Path $repoRoot "dist"
$portableDir = Join-Path $distDir "LawPDF-portable"
$zipPath = Join-Path $distDir "LawPDF-windows-portable-x64.zip"

if (Test-Path $portableDir) {
    Remove-Item -Recurse -Force $portableDir
}
New-Item -ItemType Directory -Force -Path $portableDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $portableDir "fonts") | Out-Null

Copy-Item -LiteralPath $exePath -Destination (Join-Path $portableDir $exeName) -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "vendor\pdfium.dll") -Destination (Join-Path $portableDir "pdfium.dll") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "vendor\fonts\EBGaramond.ttf") -Destination (Join-Path $portableDir "fonts\EBGaramond.ttf") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $portableDir "LICENSE") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY_NOTICES.md") -Destination (Join-Path $portableDir "THIRD_PARTY_NOTICES.md") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY_RUST_LICENSES.csv") -Destination (Join-Path $portableDir "THIRD_PARTY_RUST_LICENSES.csv") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "third_party") -Destination (Join-Path $portableDir "third_party") -Recurse -Force

if (Test-Path $zipPath) {
    Remove-Item -Force $zipPath
}
Compress-Archive -Path (Join-Path $portableDir "*") -DestinationPath $zipPath -Force

Write-Host "Portable package: $zipPath"
