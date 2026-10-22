$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assetDirectory = Join-Path $PSScriptRoot '..\assets'
New-Item -ItemType Directory -Path $assetDirectory -Force | Out-Null
$bitmap = [System.Drawing.Bitmap]::new(128, 128)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.Clear([System.Drawing.Color]::FromArgb(12, 19, 25))
$mint = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(114, 241, 189))
$pen = [System.Drawing.Pen]::new($mint, 8)
$pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$graphics.FillEllipse($mint, 49, 20, 30, 30)
$graphics.FillRectangle($mint, 49, 35, 30, 30)
$graphics.FillEllipse($mint, 49, 50, 30, 30)
$graphics.DrawArc($pen, 35, 30, 58, 64, 0, 180)
$graphics.DrawLine($pen, 64, 94, 64, 107)
$graphics.DrawLine($pen, 48, 108, 80, 108)
$bitmap.Save((Join-Path $assetDirectory 'miclift.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$png = [IO.File]::ReadAllBytes((Join-Path $assetDirectory 'miclift.png'))
$stream = [IO.File]::Create((Join-Path $assetDirectory 'miclift.ico'))
$writer = [IO.BinaryWriter]::new($stream)
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]1)
$writer.Write([byte]128); $writer.Write([byte]128); $writer.Write([byte]0); $writer.Write([byte]0)
$writer.Write([uint16]1); $writer.Write([uint16]32); $writer.Write([uint32]$png.Length); $writer.Write([uint32]22)
$writer.Write($png); $writer.Dispose()
$pen.Dispose(); $mint.Dispose(); $graphics.Dispose(); $bitmap.Dispose()
