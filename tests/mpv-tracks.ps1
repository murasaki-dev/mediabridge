param([string]$MpvPath = (Join-Path $PSScriptRoot '..\dist\package\mpv.exe'))
$ErrorActionPreference = 'Stop'
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('mediabridge-tracks-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
$files = @()
$player = $null
$previousPipe = $env:MEDIABRIDGE_TEST_TRACK_PIPE
try {
    foreach ($name in @('original.wav','alternate.wav')) {
        $file = Join-Path $fixture $name
        $files += $file
        $writer = [IO.BinaryWriter]::new([IO.File]::Create($file))
        try {
            $size = 8000 * 2 * 10
            $writer.Write([Text.Encoding]::ASCII.GetBytes('RIFF')); $writer.Write([int](36+$size))
            $writer.Write([Text.Encoding]::ASCII.GetBytes('WAVEfmt ')); $writer.Write([int]16)
            $writer.Write([int16]1); $writer.Write([int16]1); $writer.Write([int]8000); $writer.Write([int]16000)
            $writer.Write([int16]2); $writer.Write([int16]16); $writer.Write([Text.Encoding]::ASCII.GetBytes('data'))
            $writer.Write([int]$size); $writer.Write([byte[]]::new($size))
        } finally { $writer.Dispose() }
    }
    foreach ($name in @('captions.en.srt','captions.es.srt')) {
        $file = Join-Path $fixture $name; $files += $file
        [IO.File]::WriteAllText($file,"1`n00:00:00,000 --> 00:00:09,000`nFixture captions`n")
    }
    $pipeName = 'mediabridge-tracks-' + [guid]::NewGuid().ToString('N')
    $env:MEDIABRIDGE_TEST_TRACK_PIPE = '\\.\pipe\' + $pipeName
    $arguments = @('--no-config','--idle=yes','--pause=yes','--vo=null','--ao=null','--input-terminal=no',
        ('--input-ipc-server=' + $env:MEDIABRIDGE_TEST_TRACK_PIPE),
        ('--audio-file=' + $files[1]),('--sub-file=' + $files[2]),('--sub-file=' + $files[3]),$files[0])
    $quoted = $arguments | ForEach-Object { '"' + $_ + '"' }
    $player = Start-Process -FilePath (Resolve-Path -LiteralPath $MpvPath).Path -ArgumentList $quoted -WindowStyle Hidden -PassThru
    $ready = $false
    for ($attempt=0; $attempt -lt 40; $attempt++) {
        $probe = [IO.Pipes.NamedPipeClientStream]::new('.', $pipeName, [IO.Pipes.PipeDirection]::InOut)
        try { $probe.Connect(100); $ready = $true } catch {} finally { $probe.Dispose() }
        if ($ready) { break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ready) { throw 'mpv fixture IPC did not become ready' }
    Start-Sleep -Milliseconds 500
    Push-Location (Join-Path $PSScriptRoot '..')
    try {
        cargo test --locked --offline real_mpv_track_switches_and_off_preserve_loaded_lists -- --ignored
        if ($LASTEXITCODE -ne 0) { throw 'mpv track integration test failed' }
    } finally { Pop-Location }
} finally {
    $env:MEDIABRIDGE_TEST_TRACK_PIPE = $previousPipe
    if ($player -and -not $player.HasExited) { Stop-Process -Id $player.Id -Force }
    foreach ($file in $files) { if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force } }
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture }
}
