param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$Task = @('assembleDebug')
)

$ErrorActionPreference = 'Stop'
$ProjectDir = $PSScriptRoot
$ExternalRoot = if ($env:LAWPDF_ANDROID_EXTERNAL_ROOT) {
    $env:LAWPDF_ANDROID_EXTERNAL_ROOT
} else {
    'C:\tmp\lawpdf-android'
}

$env:LAWPDF_ANDROID_BUILD_DIR = Join-Path $ExternalRoot 'build'
$env:GRADLE_USER_HOME = Join-Path $ExternalRoot 'gradle-user-home'
$ProjectCache = Join-Path $ExternalRoot 'project-cache'

if (-not $env:ANDROID_HOME) {
    $BundledSdk = 'C:\Users\Arbel\android-build-tools\android-sdk'
    if (Test-Path -LiteralPath $BundledSdk) {
        $env:ANDROID_HOME = $BundledSdk
    }
}

if (-not $env:ANDROID_HOME -or -not (Test-Path -LiteralPath $env:ANDROID_HOME)) {
    throw 'Set ANDROID_HOME to an Android SDK containing platform android-35.'
}

$Wrapper = Join-Path $ProjectDir 'gradlew.bat'
if (Test-Path -LiteralPath $Wrapper) {
    $Gradle = $Wrapper
} elseif ($env:LAWPDF_GRADLE_HOME) {
    $Gradle = Join-Path $env:LAWPDF_GRADLE_HOME 'bin\gradle.bat'
} else {
    $Gradle = 'C:\Users\Arbel\android-build-tools\gradle\gradle-8.10.2\bin\gradle.bat'
}

if (-not (Test-Path -LiteralPath $Gradle)) {
    throw 'Gradle was not found. Set LAWPDF_GRADLE_HOME or generate the checked-in wrapper.'
}

New-Item -ItemType Directory -Force -Path $ExternalRoot, $env:GRADLE_USER_HOME, $ProjectCache | Out-Null
& $Gradle -p $ProjectDir --no-daemon --project-cache-dir $ProjectCache --console plain @Task
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
