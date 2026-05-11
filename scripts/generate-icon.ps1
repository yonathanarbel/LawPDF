param()

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$assetsDir = Join-Path $repoRoot "assets"
New-Item -ItemType Directory -Force -Path $assetsDir | Out-Null

Add-Type -AssemblyName System.Drawing

function New-RoundedRectPath {
    param(
        [float]$X,
        [float]$Y,
        [float]$Width,
        [float]$Height,
        [float]$Radius
    )

    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $diameter = $Radius * 2.0
    $path.AddArc($X, $Y, $diameter, $diameter, 180, 90)
    $path.AddArc($X + $Width - $diameter, $Y, $diameter, $diameter, 270, 90)
    $path.AddArc($X + $Width - $diameter, $Y + $Height - $diameter, $diameter, $diameter, 0, 90)
    $path.AddArc($X, $Y + $Height - $diameter, $diameter, $diameter, 90, 90)
    $path.CloseFigure()
    return $path
}

function New-LawPdfPngBytes {
    param([int]$Size)

    $bmp = New-Object System.Drawing.Bitmap $Size, $Size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bmp)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::ClearTypeGridFit
    $graphics.Clear([System.Drawing.Color]::Transparent)

    $scale = [float]$Size / 256.0
    $paper = New-RoundedRectPath (28 * $scale) (18 * $scale) (184 * $scale) (220 * $scale) (22 * $scale)
    $shadow = New-RoundedRectPath (34 * $scale) (24 * $scale) (184 * $scale) (220 * $scale) (22 * $scale)

    $graphics.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(44, 35, 41, 48))), $shadow)
    $graphics.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 251, 247, 237))), $paper)
    $graphics.DrawPath((New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 37, 51, 64), [Math]::Max(2.0, 5.0 * $scale))), $paper)

    $fold = New-Object System.Drawing.Drawing2D.GraphicsPath
    $fold.AddPolygon(@(
        ([System.Drawing.PointF]::new((165 * $scale), (18 * $scale))),
        ([System.Drawing.PointF]::new((212 * $scale), (65 * $scale))),
        ([System.Drawing.PointF]::new((165 * $scale), (65 * $scale)))
    ))
    $graphics.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 232, 225, 211))), $fold)
    $graphics.DrawLine((New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 37, 51, 64), [Math]::Max(1.0, 3.0 * $scale))), (165 * $scale), (18 * $scale), (212 * $scale), (65 * $scale))

    $accentPen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 193, 137, 63)), ([Math]::Max(3.0, 8.0 * $scale))
    $accentPen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $accentPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $graphics.DrawLine($accentPen, (58 * $scale), (197 * $scale), (182 * $scale), (197 * $scale))

    $format = New-Object System.Drawing.StringFormat
    $format.Alignment = [System.Drawing.StringAlignment]::Center
    $format.LineAlignment = [System.Drawing.StringAlignment]::Center

    $yFont = New-Object System.Drawing.Font "Georgia", ([Math]::Max(10.0, 124.0 * $scale)), ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $yRect = [System.Drawing.RectangleF]::new((50 * $scale), (62 * $scale), (140 * $scale), (108 * $scale))
    $graphics.DrawString("Y", $yFont, (New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 124, 30, 43))), $yRect, $format)

    if ($Size -ge 96) {
        $nameFont = New-Object System.Drawing.Font "Segoe UI Semibold", (22 * $scale), ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
        $nameRect = [System.Drawing.RectangleF]::new((48 * $scale), (167 * $scale), (144 * $scale), (28 * $scale))
        $graphics.DrawString("LawPDF", $nameFont, (New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 37, 51, 64))), $nameRect, $format)
        $nameFont.Dispose()
    }

    if ($Size -ge 192) {
        $creditFont = New-Object System.Drawing.Font "Segoe UI", (12 * $scale), ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
        $creditRect = [System.Drawing.RectangleF]::new((48 * $scale), (203 * $scale), (144 * $scale), (18 * $scale))
        $graphics.DrawString("Y. Arbel 2026", $creditFont, (New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 83, 94, 104))), $creditRect, $format)
        $creditFont.Dispose()
    }

    $stream = New-Object System.IO.MemoryStream
    $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $bytes = $stream.ToArray()

    $accentPen.Dispose()
    $format.Dispose()
    $yFont.Dispose()
    $paper.Dispose()
    $shadow.Dispose()
    $fold.Dispose()
    $graphics.Dispose()
    $bmp.Dispose()
    $stream.Dispose()

    return $bytes
}

$pngPath = Join-Path $assetsDir "lawpdf.png"
[System.IO.File]::WriteAllBytes($pngPath, (New-LawPdfPngBytes 256))

$icoPath = Join-Path $assetsDir "lawpdf.ico"
$sizes = @(16, 24, 32, 48, 64, 128, 256)
$images = @()
foreach ($size in $sizes) {
    $images += ,@{
        Size = $size
        Bytes = (New-LawPdfPngBytes $size)
    }
}

$writer = New-Object System.IO.BinaryWriter ([System.IO.File]::Open($icoPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write))
try {
    $writer.Write([UInt16]0)
    $writer.Write([UInt16]1)
    $writer.Write([UInt16]$images.Count)

    $offset = 6 + (16 * $images.Count)
    foreach ($image in $images) {
        $sizeByte = if ($image.Size -eq 256) { 0 } else { $image.Size }
        $writer.Write([byte]$sizeByte)
        $writer.Write([byte]$sizeByte)
        $writer.Write([byte]0)
        $writer.Write([byte]0)
        $writer.Write([UInt16]1)
        $writer.Write([UInt16]32)
        $writer.Write([UInt32]$image.Bytes.Length)
        $writer.Write([UInt32]$offset)
        $offset += $image.Bytes.Length
    }

    foreach ($image in $images) {
        $writer.Write([byte[]]$image.Bytes)
    }
}
finally {
    $writer.Dispose()
}

Write-Host "Wrote $pngPath"
Write-Host "Wrote $icoPath"
