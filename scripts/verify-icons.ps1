Add-Type -AssemblyName System.Drawing
$dir = 'D:\Project\Aura\icons'
foreach ($f in @('icon.png','256x256.png','32x32.png','16x16.png','32x32.rgba')) {
    $p = Join-Path $dir $f
    if ($f.EndsWith('.rgba')) {
        $len = (Get-Item $p).Length
        Write-Host "$f bytes=$len expect=4096"
        continue
    }
    $img = [System.Drawing.Image]::FromFile($p)
    Write-Host ("{0} {1}x{2}" -f $f, $img.Width, $img.Height)
    $img.Dispose()
}

$ico = Join-Path $dir 'icon.ico'
$fs = [System.IO.File]::OpenRead($ico)
$br = New-Object System.IO.BinaryReader $fs
$r = $br.ReadUInt16(); $t = $br.ReadUInt16(); $c = $br.ReadUInt16()
Write-Host "ico reserved=$r type=$t count=$c"
for ($i = 0; $i -lt $c; $i++) {
    $w = $br.ReadByte(); $h = $br.ReadByte()
    $null = $br.ReadByte(); $null = $br.ReadByte()
    $planes = $br.ReadUInt16(); $bpp = $br.ReadUInt16()
    $len = $br.ReadUInt32(); $off = $br.ReadUInt32()
    Write-Host ("  entry {0} {1}x{2} bpp={3} len={4} off={5}" -f $i, $w, $h, $bpp, $len, $off)
}
$fs.Close()
