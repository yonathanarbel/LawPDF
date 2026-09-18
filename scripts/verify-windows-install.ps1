param(
    [Parameter(Mandatory=$true)][string]$Installer,
    [Parameter(Mandatory=$true)][string]$ExpectedVersion,
    [Parameter(Mandatory=$true)][string]$ExpectedSha256,
    [string]$InstallDirectory = 'C:\tmp\lawpdf-installed',
    [string]$EvidenceDirectory = 'C:\tmp\lawpdf-install-evidence'
)
$ErrorActionPreference = 'Stop'
if (-not $env:CI) { throw 'This installation verifier is intended for an isolated CI Windows machine.' }
function Canonical-Version([string]$Value) {
    $version = [Version]$Value
    return '{0}.{1}.{2}.{3}' -f $version.Major, $version.Minor, [Math]::Max(0, $version.Build), [Math]::Max(0, $version.Revision)
}
$actualHash = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash
if ($actualHash -ne $ExpectedSha256) { throw 'Installer checksum mismatch.' }
$arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', ('/DIR="{0}"' -f $InstallDirectory))
$install = Start-Process -FilePath (Resolve-Path $Installer) -ArgumentList $arguments -Wait -PassThru
if ($install.ExitCode -ne 0) { throw "Installer exited with $($install.ExitCode)." }
$executable = Join-Path $InstallDirectory 'lawpdf.exe'
$info = (Get-Item -LiteralPath $executable).VersionInfo
$expected = Canonical-Version $ExpectedVersion
if ((Canonical-Version $info.ProductVersion) -ne $expected -or (Canonical-Version $info.FileVersion) -ne $expected) {
    throw 'Installed product/file version does not match the release.'
}
$shell = New-Object -ComObject WScript.Shell
$shortcuts = @(
    (Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'LawPDF\LawPDF.lnk'),
    (Join-Path ([Environment]::GetFolderPath('Programs')) 'LawPDF\LawPDF.lnk')
) | Where-Object { Test-Path -LiteralPath $_ }
if ($shortcuts.Count -eq 0) { throw 'The Start Menu shortcut was not installed.' }
foreach ($shortcut in $shortcuts) {
    $target = $shell.CreateShortcut($shortcut).TargetPath
    if ([IO.Path]::GetFullPath($target) -ne [IO.Path]::GetFullPath($executable)) { throw 'A Start Menu shortcut targets a different LawPDF executable.' }
}
New-Item -ItemType Directory -Force -Path $EvidenceDirectory | Out-Null
# PowerShell can return immediately when directly invoking a GUI-subsystem EXE.
# Wait for this exact process and retain its streams before interpreting the result.
$statusPath = Join-Path $EvidenceDirectory 'runtime-status.json'
$errorPath = Join-Path $EvidenceDirectory 'runtime-stderr.log'
$runtime = Start-Process -FilePath $executable `
    -ArgumentList @('--lm2-runtime-status', '--require-native', '--require-context', '--require-arbiter', '--require-note-head', '--require-link-ranker') `
    -NoNewWindow -PassThru -Wait -RedirectStandardOutput $statusPath -RedirectStandardError $errorPath
if ($runtime.ExitCode -ne 0) { throw "Installed runtime verification exited with $($runtime.ExitCode); see $errorPath." }
$status = Get-Content -LiteralPath $statusPath -Raw -Encoding utf8 | ConvertFrom-Json
if ($status.requirements_met -ne $true) { throw 'Installed runtime requirements were not met.' }
$evidence = [ordered]@{
    installer_sha256 = $actualHash.ToLowerInvariant()
    product_version = $info.ProductVersion
    file_version = $info.FileVersion
    executable = $executable
    start_menu_shortcuts = $shortcuts
    runtime_exit_code = $runtime.ExitCode
    runtime_requirements_met = $status.requirements_met
    authenticode_status = (Get-AuthenticodeSignature -FilePath $Installer).Status.ToString()
}
$evidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'installation.json') -Encoding utf8
$evidence | ConvertTo-Json -Depth 5
Get-Content -LiteralPath $statusPath -Raw -Encoding utf8
