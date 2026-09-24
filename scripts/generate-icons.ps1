# Generate EnvBox icon set from a source PNG (Veya icons/ layout).
param(
    [Parameter(Mandatory = $true)][string]$Source,
    [string]$OutDir = "icons"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

if (-not (Test-Path -LiteralPath $Source)) {
    throw "source not found: $Source"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$src = [System.Drawing.Image]::FromFile((Resolve-Path -LiteralPath $Source))
try {
    Write-Host "source $($src.Width)x$($src.Height)"

    function Save-Resized([int]$size, [string]$name) {
        $bmp = New-Object System.Drawing.Bitmap $size, $size
        try {
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            try {
                $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $g.Clear([System.Drawing.Color]::Transparent)
                $g.DrawImage($src, 0, 0, $size, $size)
            } finally {
                $g.Dispose()
            }
            $path = Join-Path $OutDir $name
            $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
            Write-Host "  $name $((Get-Item $path).Length)"
        } finally {
            $bmp.Dispose()
        }
    }

    Save-Resized 512 "icon.png"
    Save-Resized 512 "512x512.png"
    Save-Resized 256 "256x256.png"
    Save-Resized 256 "128x128@2x.png"
    Save-Resized 128 "128x128.png"
    Save-Resized 64 "64x64.png"
    Save-Resized 48 "48x48.png"
    Save-Resized 32 "32x32.png"
    Save-Resized 24 "24x24.png"
    Save-Resized 16 "16x16.png"

    # 32x32.rgba — raw top-down RGBA
    $b32 = New-Object System.Drawing.Bitmap 32, 32
    try {
        $g = [System.Drawing.Graphics]::FromImage($b32)
        try {
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $g.Clear([System.Drawing.Color]::Transparent)
            $g.DrawImage($src, 0, 0, 32, 32)
        } finally {
            $g.Dispose()
        }
        $rect = New-Object System.Drawing.Rectangle 0, 0, 32, 32
        $data = $b32.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $raw = New-Object byte[] (32 * 32 * 4)
            [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $raw, 0, $raw.Length)
            # Convert BGRA -> RGBA
            for ($i = 0; $i -lt $raw.Length; $i += 4) {
                $b = $raw[$i]; $r = $raw[$i + 2]
                $raw[$i] = $r; $raw[$i + 2] = $b
            }
            [System.IO.File]::WriteAllBytes((Join-Path $OutDir "32x32.rgba"), $raw)
            Write-Host "  32x32.rgba $($raw.Length)"
        } finally {
            $b32.UnlockBits($data)
        }
    } finally {
        $b32.Dispose()
    }

    # icon.ico — multi-size PNG-in-ICO
    function Save-Ico {
        param([int[]]$Sizes, [string]$Path)
        $pngs = @()
        foreach ($s in $Sizes) {
            $bmp = New-Object System.Drawing.Bitmap $s, $s
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            try {
                $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $g.Clear([System.Drawing.Color]::Transparent)
                $g.DrawImage($src, 0, 0, $s, $s)
            } finally {
                $g.Dispose()
            }
            $ms = New-Object System.IO.MemoryStream
            $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
            $bmp.Dispose()
            $pngs += ,@{ Size = $s; Bytes = $ms.ToArray() }
        }

        $count = $pngs.Count
        $header = New-Object System.IO.MemoryStream
        $bw = New-Object System.IO.BinaryWriter $header
        $bw.Write([UInt16]0)
        $bw.Write([UInt16]1)
        $bw.Write([UInt16]$count)
        $offset = 6 + ($count * 16)
        foreach ($e in $pngs) {
            $dim = if ($e.Size -ge 256) { 0 } else { $e.Size }
            $bw.Write([byte]$dim)
            $bw.Write([byte]$dim)
            $bw.Write([byte]0)
            $bw.Write([byte]0)
            $bw.Write([UInt16]1)
            $bw.Write([UInt16]32)
            $bw.Write([UInt32]$e.Bytes.Length)
            $bw.Write([UInt32]$offset)
            $offset += $e.Bytes.Length
        }
        $out = New-Object System.IO.MemoryStream
        $out.Write($header.ToArray(), 0, $header.ToArray().Length)
        foreach ($e in $pngs) {
            $out.Write($e.Bytes, 0, $e.Bytes.Length)
        }
        [System.IO.File]::WriteAllBytes($Path, $out.ToArray())
        Write-Host "  icon.ico $($out.ToArray().Length)"
    }

    Save-Ico -Sizes @(16, 24, 32, 48, 64, 128, 256) -Path (Join-Path $OutDir "icon.ico")

    Get-ChildItem -LiteralPath $OutDir | Sort-Object Name | ForEach-Object {
        Write-Host ("{0}`t{1}" -f $_.Name, $_.Length)
    }
} finally {
    $src.Dispose()
}
