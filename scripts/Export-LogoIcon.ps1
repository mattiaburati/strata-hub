param([string]$Source = (Join-Path $PSScriptRoot '..\assets\strata-hub-logo.png'),
      [string]$Destination = (Join-Path $PSScriptRoot '..\assets\strata-hub.ico'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$image = [Drawing.Image]::FromFile([IO.Path]::GetFullPath($Source))
$frames = [Collections.Generic.List[object]]::new()
try {
    foreach ($size in @(16, 24, 32, 40, 48, 64, 128, 256)) {
        $bitmap = [Drawing.Bitmap]::new($size, $size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $stream = [IO.MemoryStream]::new()
        try {
            $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.DrawImage($image, [Drawing.Rectangle]::new(0, 0, $size, $size))
            $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
            $frames.Add(@{Size=$size; Data=$stream.ToArray()})
        } finally { $stream.Dispose(); $graphics.Dispose(); $bitmap.Dispose() }
    }
    $output = [IO.BinaryWriter]::new([IO.File]::Create([IO.Path]::GetFullPath($Destination)))
    try {
        $output.Write([uint16]0); $output.Write([uint16]1); $output.Write([uint16]$frames.Count)
        $offset = 6 + 16 * $frames.Count
        foreach ($frame in $frames) {
            $dimension = if ($frame.Size -eq 256) { 0 } else { $frame.Size }
            $output.Write([byte]$dimension); $output.Write([byte]$dimension)
            $output.Write([byte]0); $output.Write([byte]0)
            $output.Write([uint16]1); $output.Write([uint16]32)
            $output.Write([uint32]$frame.Data.Length); $output.Write([uint32]$offset)
            $offset += $frame.Data.Length
        }
        foreach ($frame in $frames) { $output.Write([byte[]]$frame.Data) }
    } finally { $output.Dispose() }
} finally { $image.Dispose() }
