param(
    [string]$Version = "1.2.10"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$destinationDir = Join-Path $repoRoot "profile-models\lm2-native-catboost-runtime"
$destination = Join-Path $destinationDir "catboostmodel.dll"
$expectedVersion = "1.2.10"
$expectedSha256 = "668D8867940A0832ACCAAD6004E249F11ECC48EB63EB1D65551CFDD37C1939AD"

if ($Version -ne $expectedVersion) {
    throw "No pinned CatBoost checksum is registered for $Version (expected $expectedVersion)."
}

$url = "https://github.com/catboost/catboost/releases/download/v$Version/catboostmodel.dll"
New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null

if (Test-Path -LiteralPath $destination) {
    $actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
    if ($actual -eq $expectedSha256) {
        Write-Host "Pinned CatBoost runtime already present: $destination"
        exit 0
    }
    Remove-Item -LiteralPath $destination -Force
}

Write-Host "Downloading pinned CatBoost $Version Windows runtime..."
Invoke-WebRequest -Uri $url -OutFile $destination
$actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
if ($actual -ne $expectedSha256) {
    Remove-Item -LiteralPath $destination -Force
    throw "CatBoost DLL checksum mismatch: expected $expectedSha256, got $actual"
}

Write-Host "Verified CatBoost runtime: $destination ($actual)"
