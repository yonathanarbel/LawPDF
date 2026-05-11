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

    $bell = New-Object System.Drawing.Drawing2D.GraphicsPath
    $bell.StartFigure()
    $bell.AddBezier(
        [System.Drawing.PointF]::new((82 * $scale), (187 * $scale)),
        [System.Drawing.PointF]::new((88 * $scale), (162 * $scale)),
        [System.Drawing.PointF]::new((76 * $scale), (130 * $scale)),
        [System.Drawing.PointF]::new((78 * $scale), (94 * $scale))
    )
    $bell.AddBezier(
        [System.Drawing.PointF]::new((78 * $scale), (94 * $scale)),
        [System.Drawing.PointF]::new((80 * $scale), (58 * $scale)),
        [System.Drawing.PointF]::new((100 * $scale), (38 * $scale)),
        [System.Drawing.PointF]::new((128 * $scale), (38 * $scale))
    )
    $bell.AddBezier(
        [System.Drawing.PointF]::new((128 * $scale), (38 * $scale)),
        [System.Drawing.PointF]::new((156 * $scale), (38 * $scale)),
        [System.Drawing.PointF]::new((176 * $scale), (58 * $scale)),
        [System.Drawing.PointF]::new((178 * $scale), (94 * $scale))
    )
    $bell.AddBezier(
        [System.Drawing.PointF]::new((178 * $scale), (94 * $scale)),
        [System.Drawing.PointF]::new((180 * $scale), (130 * $scale)),
        [System.Drawing.PointF]::new((168 * $scale), (162 * $scale)),
        [System.Drawing.PointF]::new((174 * $scale), (187 * $scale))
    )
    $bell.AddBezier(
        [System.Drawing.PointF]::new((174 * $scale), (187 * $scale)),
        [System.Drawing.PointF]::new((156 * $scale), (202 * $scale)),
        [System.Drawing.PointF]::new((100 * $scale), (202 * $scale)),
        [System.Drawing.PointF]::new((82 * $scale), (187 * $scale))
    )
    $bell.CloseFigure()

    $shadow = $bell.Clone()
    $matrix = New-Object System.Drawing.Drawing2D.Matrix
    $matrix.Translate((7 * $scale), (9 * $scale))
    $shadow.Transform($matrix)

    $handlePen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 37, 51, 64)), ([Math]::Max(2.0, 8.0 * $scale))
    $handlePen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $handlePen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $rimPen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 37, 51, 64)), ([Math]::Max(2.0, 7.0 * $scale))
    $rimPen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $rimPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $outlinePen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 37, 51, 64)), ([Math]::Max(2.0, 6.0 * $scale))

    $shadowBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(44, 35, 41, 48))
    $bellBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 251, 247, 237))
    $clapperBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 193, 137, 63))
    $letterBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 124, 30, 43))

    $graphics.FillPath($shadowBrush, $shadow)
    $graphics.FillPath($bellBrush, $bell)
    $graphics.DrawPath($outlinePen, $bell)
    $graphics.DrawArc(
        $handlePen,
        [System.Drawing.RectangleF]::new((101 * $scale), (24 * $scale), (54 * $scale), (40 * $scale)),
        205,
        130
    )
    $graphics.DrawLine($rimPen, (67 * $scale), (188 * $scale), (189 * $scale), (188 * $scale))
    $graphics.FillEllipse($clapperBrush, (111 * $scale), (190 * $scale), (34 * $scale), (34 * $scale))
    $graphics.DrawEllipse($outlinePen, (111 * $scale), (190 * $scale), (34 * $scale), (34 * $scale))

    $format = New-Object System.Drawing.StringFormat
    $format.Alignment = [System.Drawing.StringAlignment]::Center
    $format.LineAlignment = [System.Drawing.StringAlignment]::Center

    $aFont = New-Object System.Drawing.Font "Georgia", ([Math]::Max(9.0, 102.0 * $scale)), ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $aRect = [System.Drawing.RectangleF]::new((76 * $scale), (74 * $scale), (104 * $scale), (88 * $scale))
    $graphics.DrawString("A", $aFont, $letterBrush, $aRect, $format)

    $stream = New-Object System.IO.MemoryStream
    $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $bytes = $stream.ToArray()

    $aFont.Dispose()
    $format.Dispose()
    $letterBrush.Dispose()
    $clapperBrush.Dispose()
    $bellBrush.Dispose()
    $shadowBrush.Dispose()
    $outlinePen.Dispose()
    $rimPen.Dispose()
    $handlePen.Dispose()
    $matrix.Dispose()
    $shadow.Dispose()
    $bell.Dispose()
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
