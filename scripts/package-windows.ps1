param(
    [ValidateSet("debug", "release")]
    [string]$Configuration = "release"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot "target" }
$targetDir = Join-Path $targetRoot $Configuration
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

$nativeRuntimeSource = Join-Path $repoRoot "profile-models\lm2-native-catboost-runtime"
$contextRuntimeSource = Join-Path $repoRoot "profile-models\lm2-context-twopass-runtime"
$nativeModelName = "lm2-catboost-augmented-epoch51lv-relabels-tc.cbm"
$nativeLibraryName = "catboostmodel.dll"
$contextModelName = "lm2-context-twopass-hgb-v1.json"
$requiredRuntimeAssets = @(
    (Join-Path $nativeRuntimeSource $nativeModelName),
    (Join-Path $nativeRuntimeSource $nativeLibraryName),
    (Join-Path $contextRuntimeSource $contextModelName)
)
foreach ($asset in $requiredRuntimeAssets) {
    if (-not (Test-Path -LiteralPath $asset)) {
        throw "Missing promoted LM2 runtime asset: $asset. Run scripts\fetch-catboost-windows.ps1 first."
    }
}

Copy-Item -LiteralPath $exePath -Destination (Join-Path $portableDir $exeName) -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "assets\lawpdf.ico") -Destination (Join-Path $portableDir "lawpdf.ico") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "vendor\pdfium.dll") -Destination (Join-Path $portableDir "pdfium.dll") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "vendor\fonts\EBGaramond.ttf") -Destination (Join-Path $portableDir "fonts\EBGaramond.ttf") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $portableDir "LICENSE") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY_NOTICES.md") -Destination (Join-Path $portableDir "THIRD_PARTY_NOTICES.md") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY_RUST_LICENSES.csv") -Destination (Join-Path $portableDir "THIRD_PARTY_RUST_LICENSES.csv") -Force
Copy-Item -LiteralPath (Join-Path $repoRoot "third_party") -Destination (Join-Path $portableDir "third_party") -Recurse -Force

$nativeRuntimeDest = Join-Path $portableDir "profile-models\lm2-native-catboost-runtime"
$contextRuntimeDest = Join-Path $portableDir "profile-models\lm2-context-twopass-runtime"
New-Item -ItemType Directory -Force -Path $nativeRuntimeDest | Out-Null
New-Item -ItemType Directory -Force -Path $contextRuntimeDest | Out-Null
Copy-Item -LiteralPath (Join-Path $nativeRuntimeSource $nativeModelName) -Destination (Join-Path $nativeRuntimeDest $nativeModelName) -Force
Copy-Item -LiteralPath (Join-Path $nativeRuntimeSource $nativeLibraryName) -Destination (Join-Path $nativeRuntimeDest $nativeLibraryName) -Force
Copy-Item -LiteralPath (Join-Path $contextRuntimeSource $contextModelName) -Destination (Join-Path $contextRuntimeDest $contextModelName) -Force

$verifyWorkingDir = Join-Path ([System.IO.Path]::GetTempPath()) "lawpdf-package-runtime-verify"
New-Item -ItemType Directory -Force -Path $verifyWorkingDir | Out-Null
Push-Location $verifyWorkingDir
try {
    & (Join-Path $portableDir $exeName) --lm2-runtime-status --require-native --require-context
    if ($LASTEXITCODE -ne 0) {
        throw "Packaged LawPDF did not load the promoted native CatBoost + context runtime."
    }
}
finally {
    Pop-Location
}

if (Test-Path $zipPath) {
    Remove-Item -Force $zipPath
}
Compress-Archive -Path (Join-Path $portableDir "*") -DestinationPath $zipPath -Force

Write-Host "Portable package: $zipPath"
