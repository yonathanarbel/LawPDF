param(
    [Parameter(Mandatory=$true)][string]$PackageDirectory,
    [Parameter(Mandatory=$true)][string]$ExpectedAppVersion
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
# Never add a test certificate to a maintainer or customer machine.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Test certificate installation is restricted to disposable GitHub-hosted runners.' }
$evidencePath=Join-Path $PackageDirectory 'package-evidence.json'
$evidence=Get-Content -LiteralPath $evidencePath -Raw | ConvertFrom-Json
$unsigned=$evidence.package_path
if ($evidence.app_version -ne $ExpectedAppVersion) { throw 'Unexpected built app version.' }
if ((Get-FileHash -LiteralPath $unsigned -Algorithm SHA256).Hash.ToLowerInvariant() -ne $evidence.package_sha256) { throw 'Unsigned package changed after verification.' }
$signed=Join-Path $env:RUNNER_TEMP 'LawPDF-CI-install-only.msix'
Copy-Item -LiteralPath $unsigned -Destination $signed
$cert=New-SelfSignedCertificate -Type Custom -Subject $evidence.publisher -KeyUsage DigitalSignature -FriendlyName 'LawPDF disposable CI validation only' -CertStoreLocation Cert:\CurrentUser\My -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3','2.5.29.19={text}')
$cer=Join-Path $env:RUNNER_TEMP 'LawPDF-CI-install-only.cer'
$installed=$null
try {
    Export-Certificate -Cert $cert -FilePath $cer | Out-Null
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    $kitRoot=Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $signtool=Get-ChildItem -LiteralPath $kitRoot -Directory | Sort-Object Name -Descending | ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $signtool) { throw 'Windows SDK SignTool missing.' }
    & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $signed
    if ($LASTEXITCODE -ne 0) { throw 'Disposable test signing failed.' }
    & $signtool verify /pa $signed
    if ($LASTEXITCODE -ne 0) { throw 'Test signature verification failed.' }
    $vc=Get-AppxPackage -Name Microsoft.VCLibs.140.00.UWPDesktop | Where-Object { $_.Architecture -eq 'X64' -and [version]$_.Version -ge [version]'14.0.30704.0' } | Select-Object -First 1
    if (-not $vc) {
        $vcPath=Join-Path $env:RUNNER_TEMP 'Microsoft.VCLibs.x64.14.00.Desktop.appx'
        Invoke-WebRequest -Uri 'https://aka.ms/Microsoft.VCLibs.x64.14.00.Desktop.appx' -OutFile $vcPath
        # AppX deployment validates Microsoft's framework signature.
        Add-AppxPackage -Path $vcPath
    }
    Add-AppxPackage -Path $signed
    $installed=Get-AppxPackage -Name $evidence.identity_name
    if (-not $installed -or $installed.Version -ne $evidence.package_version) { throw 'MSIX installation identity/version mismatch.' }
    $exe=Join-Path $installed.InstallLocation 'lawpdf.exe'
    if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $evidence.executable_sha256) { throw 'Installed MSIX executable mismatch.' }
    $versionInfo=(Get-Item -LiteralPath $exe).VersionInfo
    foreach ($value in @($versionInfo.ProductVersion,$versionInfo.FileVersion)) { if ($value -notin @($ExpectedAppVersion,"$ExpectedAppVersion.0")) { throw 'Installed executable product/file version mismatch.' } }
    $distributionFile=Join-Path $PackageDirectory 'installed-distribution.json'
    $process=Start-Process $exe -ArgumentList '--distribution-status' -Wait -PassThru -NoNewWindow -RedirectStandardOutput $distributionFile
    if ($process.ExitCode -ne 0) { throw 'Installed distribution status failed.' }
    $distribution=Get-Content -LiteralPath $distributionFile -Raw | ConvertFrom-Json
    if ($distribution.channel -ne 'microsoft-store' -or $distribution.self_updates_enabled -ne $false) { throw 'Installed package would use the wrong update channel.' }
    if ($distribution.package_identity -ne $installed.PackageFullName) { throw 'Process is not running with the installed MSIX identity.' }
    $runtimeFile=Join-Path $PackageDirectory 'installed-runtime.json'
    $process=Start-Process $exe -ArgumentList @('--lm2-runtime-status','--require-native','--require-context','--require-arbiter','--require-note-head','--require-link-ranker') -Wait -PassThru -NoNewWindow -RedirectStandardOutput $runtimeFile
    $runtime=Get-Content -LiteralPath $runtimeFile -Raw | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or $runtime.requirements_met -ne $true -or $runtime.app_version -ne $ExpectedAppVersion) { throw 'Installed MSIX runtime verification failed.' }
    $installPrefix=$installed.InstallLocation.TrimEnd('\')+'\'
    foreach ($field in @('fasttab_model_path','native_model_path','native_library_path','context_model_path','context_arbiter_model_path','note_head_model_path','link_ranker_model_path')) {
        $reportedPath=[string]$runtime.$field
        if ([string]::IsNullOrWhiteSpace($reportedPath) -or -not [IO.Path]::IsPathFullyQualified($reportedPath)) { throw "Installed runtime did not report an absolute path for $field." }
        $path=[IO.Path]::GetFullPath($reportedPath)
        if (-not $path.StartsWith($installPrefix,[StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Runtime asset is missing or outside the installed package: $field" }
    }
    $aumid="$($installed.PackageFamilyName)!LawPDF"
    $start=Get-StartApps | Where-Object AppID -eq $aumid
    if (-not $start) { throw 'Installed app missing from Start Menu.' }
    $manifest=Get-AppxPackageManifest -Package $installed.PackageFullName
    $namespaces=[Xml.XmlNamespaceManager]::new($manifest.NameTable)
    $namespaces.AddNamespace('uap','http://schemas.microsoft.com/appx/manifest/uap/windows10')
    if (-not $manifest.SelectSingleNode('//uap:FileTypeAssociation/uap:SupportedFileTypes/uap:FileType[text()=".pdf"]',$namespaces)) { throw 'Packaged PDF association missing.' }
    [ordered]@{
        schema='lawpdf-msix-installation-v1'; app_version=$ExpectedAppVersion;
        package_version=$installed.Version; package_full_name=$installed.PackageFullName;
        product_version=$versionInfo.ProductVersion; file_version=$versionInfo.FileVersion;
        executable_sha256=$evidence.executable_sha256; unsigned_package_sha256=$evidence.package_sha256;
        runtime_exit_code=$process.ExitCode; runtime_requirements_met=$runtime.requirements_met;
        start_menu_app_id=$aumid; pdf_association=$true; self_updates_enabled=$false;
        installation_scope='Disposable hosted Windows runner with temporary CI test certificate';
        microsoft_store_approval=$false; fresh_store_install_tested=$false
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $PackageDirectory 'installation-evidence.json') -Encoding utf8
    Write-Host 'Exact MSIX payload installation, identity, versions, Start Menu, PDF association and runtime verified.'
} finally {
    if ($installed) { Remove-AppxPackage -Package $installed.PackageFullName -ErrorAction Continue }
    Remove-Item -LiteralPath ("Cert:\LocalMachine\TrustedPeople\"+$cert.Thumbprint) -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath ("Cert:\CurrentUser\My\"+$cert.Thumbprint) -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $signed,$cer -Force -ErrorAction SilentlyContinue
}
