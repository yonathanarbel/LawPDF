param(
    [Parameter(Mandatory=$true)][string]$PackageDirectory,
    [Parameter(Mandatory=$true)][string]$SamplePdf,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'This capture is restricted to disposable GitHub-hosted Windows runners.'
}
$evidence = Get-Content (Join-Path $PackageDirectory 'package-evidence.json') -Raw | ConvertFrom-Json
$package = Join-Path $PackageDirectory (Split-Path $evidence.package_path -Leaf)
if ((Get-FileHash $package -Algorithm SHA256).Hash.ToLowerInvariant() -ne $evidence.package_sha256) {
    throw 'Submission package checksum mismatch.'
}
$payload = Join-Path $env:RUNNER_TEMP 'lawpdf-screenshot-payload'
[IO.Compression.ZipFile]::ExtractToDirectory($package, $payload)
$exe = Join-Path $payload 'lawpdf.exe'
if ((Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $evidence.executable_sha256) {
    throw 'Submission executable checksum mismatch.'
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class LawPdfCapture {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int w, int h, bool repaint);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
'@
[LawPdfCapture]::SetProcessDPIAware() | Out-Null
$process = $null
try {
    $process = Start-Process -FilePath $exe -ArgumentList ('"' + $SamplePdf + '"') -WorkingDirectory $payload -PassThru -RedirectStandardError (Join-Path $OutputDirectory 'launch-errors.txt')
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        Start-Sleep -Milliseconds 500
        $process.Refresh()
        if ($process.HasExited) { throw 'LawPDF exited before a screenshot could be taken.' }
    } until ($process.MainWindowHandle -ne [IntPtr]::Zero -or [DateTime]::UtcNow -ge $deadline)
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) { throw 'No visible LawPDF window was created.' }
    $screen = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    $width = [Math]::Min(1440, $screen.Width)
    $height = [Math]::Min(1000, $screen.Height)
    if ($width -lt 1024 -or $height -lt 720) { throw 'Runner desktop is too small for a useful Store screenshot.' }
    [LawPdfCapture]::ShowWindow($process.MainWindowHandle, 9) | Out-Null
    [LawPdfCapture]::MoveWindow($process.MainWindowHandle, 0, 0, $width, $height, $true) | Out-Null
    [LawPdfCapture]::SetForegroundWindow($process.MainWindowHandle) | Out-Null
    # Only an original synthetic PDF is opened; no credentials or network features are configured.
    Start-Sleep -Seconds 15
    $rect = New-Object LawPdfCapture+RECT
    if (-not [LawPdfCapture]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) { throw 'Cannot read the app window bounds.' }
    $x = [Math]::Max($screen.Left, $rect.Left)
    $y = [Math]::Max($screen.Top, $rect.Top)
    $w = [Math]::Min($screen.Right, $rect.Right) - $x
    $h = [Math]::Min($screen.Bottom, $rect.Bottom) - $y
    $bitmap = New-Object Drawing.Bitmap($w, $h)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($x, $y, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $OutputDirectory 'LawPDF-Windows-reading.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    [ordered]@{
        app_version=$evidence.app_version; package_sha256=$evidence.package_sha256;
        executable_sha256=$evidence.executable_sha256; sample_sha256=(Get-FileHash $SamplePdf -Algorithm SHA256).Hash.ToLowerInvariant();
        capture='Actual Windows app window, unmodified pixels'; width=$w; height=$h;
        scope='Unsigned submission payload extracted on a disposable runner. Not a Store-install test.';
        visual_review_required=$true
    } | ConvertTo-Json | Set-Content (Join-Path $OutputDirectory 'screenshot-evidence.json') -Encoding utf8
} finally {
    if ($process -and -not $process.HasExited) {
        $process.CloseMainWindow() | Out-Null
        if (-not $process.WaitForExit(5000)) { $process.Kill(); $process.WaitForExit() }
    }
}
