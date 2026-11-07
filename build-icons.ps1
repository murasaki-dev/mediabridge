$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = $PSScriptRoot
$assets = Join-Path $root 'assets'
$source = [Drawing.Bitmap]::FromFile((Join-Path $assets 'mediabridge-logo-source.png'))
try {
    # Normalize the approved artwork's margins without changing its design.
    $left=$source.Width; $top=$source.Height; $right=0; $bottom=0
    for($y=0;$y -lt $source.Height;$y++) {
        for($x=0;$x -lt $source.Width;$x++) {
            $pixel=$source.GetPixel($x,$y)
            if($pixel.R -lt 100 -and $pixel.G -lt 100 -and $pixel.B -lt 100) {
                $left=[Math]::Min($left,$x); $right=[Math]::Max($right,$x)
                $top=[Math]::Min($top,$y); $bottom=[Math]::Max($bottom,$y)
            }
        }
    }
    $bounds=[Drawing.Rectangle]::FromLTRB($left,$top,$right+1,$bottom+1)
    function Render-Icon([int]$size) {
        $bitmap=[Drawing.Bitmap]::new($size,$size,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics=[Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.Clear([Drawing.Color]::White)
            $graphics.InterpolationMode=[Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode=[Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $scale=($size*0.84)/[Math]::Max($bounds.Width,$bounds.Height)
            $width=[int][Math]::Round($bounds.Width*$scale); $height=[int][Math]::Round($bounds.Height*$scale)
            $target=[Drawing.Rectangle]::new([int](($size-$width)/2),[int](($size-$height)/2),$width,$height)
            $graphics.DrawImage($source,$target,$bounds,[Drawing.GraphicsUnit]::Pixel)
        } finally { $graphics.Dispose() }
        return ,$bitmap
    }
    $master=Render-Icon 512
    try { $master.Save((Join-Path $assets 'mediabridge.png'),[Drawing.Imaging.ImageFormat]::Png) } finally { $master.Dispose() }
    $entries=@()
    foreach($size in @(16,24,32,48,64,128,256)) {
        $bitmap=Render-Icon $size; $memory=[IO.MemoryStream]::new()
        try {
            $bitmap.Save($memory,[Drawing.Imaging.ImageFormat]::Png)
            $entries+=@{Size=$size; Bytes=$memory.ToArray()}
            if($size -eq 32) {
                $rgba=[byte[]]::new(32*32*4)
                for($y=0;$y -lt 32;$y++){for($x=0;$x -lt 32;$x++){
                    $pixel=$bitmap.GetPixel($x,$y); $i=($y*32+$x)*4
                    $rgba[$i]=$pixel.R; $rgba[$i+1]=$pixel.G; $rgba[$i+2]=$pixel.B; $rgba[$i+3]=$pixel.A
                }}
                [IO.File]::WriteAllBytes((Join-Path $assets 'tray-32.rgba'),$rgba)
            }
        } finally { $memory.Dispose(); $bitmap.Dispose() }
    }
    $writer=[IO.BinaryWriter]::new([IO.File]::Create((Join-Path $assets 'mediabridge.ico')))
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$entries.Count)
        $offset=6+16*$entries.Count
        foreach($entry in $entries){
            $dimension=if($entry.Size -eq 256){0}else{$entry.Size}
            $writer.Write([byte]$dimension);$writer.Write([byte]$dimension);$writer.Write([byte]0);$writer.Write([byte]0)
            $writer.Write([uint16]1);$writer.Write([uint16]32);$writer.Write([uint32]$entry.Bytes.Length);$writer.Write([uint32]$offset)
            $offset+=$entry.Bytes.Length
        }
        foreach($entry in $entries){$writer.Write([byte[]]$entry.Bytes)}
    } finally { $writer.Dispose() }
    $png=[Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $assets 'mediabridge.png')))
    $svg='<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="512" height="512" viewBox="0 0 512 512"><image width="512" height="512" xlink:href="data:image/png;base64,'+$png+'"/></svg>'
    [IO.File]::WriteAllText((Join-Path $root 'packaging/flatpak/io.github.murasaki_dev.MediaBridge.svg'),$svg)
} finally { $source.Dispose() }

