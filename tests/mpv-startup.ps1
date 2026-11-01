param([string]$MpvPath = (Join-Path $PSScriptRoot '..\dist\package\mpv.exe'))
$ErrorActionPreference='Stop'
$directory=Join-Path ([IO.Path]::GetTempPath()) ('mediabridge-startup-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory | Out-Null
$wave=Join-Path $directory 'fixture.wav'; $script=Join-Path $directory 'delay.lua'; $log=Join-Path $directory 'mpv.log'
$player=$null
$pipe='mediabridge-startup-'+[guid]::NewGuid().ToString('N')
function Invoke-Mpv($command) {
    $stream=[IO.Pipes.NamedPipeClientStream]::new('.',$pipe,[IO.Pipes.PipeDirection]::InOut)
    try {
        $stream.Connect(250)
        $writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false),1024,$true)
        $reader=[IO.StreamReader]::new($stream,[Text.Encoding]::UTF8,$false,1024,$true)
        try {
            $writer.WriteLine((@{command=$command;request_id=1}|ConvertTo-Json -Compress -Depth 5));$writer.Flush()
            $task=$reader.ReadLineAsync()
            if(-not $task.Wait(1500)){throw 'mpv IPC response timed out'}
            return ($task.Result|ConvertFrom-Json)
        }finally{$reader.Dispose();$writer.Dispose()}
    }finally{$stream.Dispose()}
}
try {
    $writer=[IO.BinaryWriter]::new([IO.File]::Create($wave))
    try {
        $size=8000*2*10
        $writer.Write([Text.Encoding]::ASCII.GetBytes('RIFF'));$writer.Write([int](36+$size))
        $writer.Write([Text.Encoding]::ASCII.GetBytes('WAVEfmt '));$writer.Write([int]16)
        $writer.Write([int16]1);$writer.Write([int16]1);$writer.Write([int]8000);$writer.Write([int]16000)
        $writer.Write([int16]2);$writer.Write([int16]16);$writer.Write([Text.Encoding]::ASCII.GetBytes('data'))
        $writer.Write([int]$size);$writer.Write([byte[]]::new($size))
    }finally{$writer.Dispose()}
    # Simulate an asynchronous extractor hook without network or yt-dlp.
    [IO.File]::WriteAllText($script,"mp.add_hook('on_load', 5, function(h) h:defer(); mp.add_timeout(4, function() h:cont() end) end)")
    $source=[IO.File]::ReadAllText((Join-Path $PSScriptRoot '../main.rs'))
    $osd=[regex]::Match($source,'"--osd-msg1=([^"\r\n]+)"').Groups[1].Value
    $force=[regex]::Match($source,'"(--force-window=immediate)"').Groups[1].Value
    if(-not $osd -or -not $force){throw 'Startup options missing from source'}
    $arguments=@('--no-config','--idle=yes','--pause=yes','--ao=null','--osd-level=1',$force,
        "--osd-msg1=$osd","--script=$script","--log-file=$log",('--input-ipc-server=\\.\pipe\'+$pipe),$wave)
    $quoted=$arguments|ForEach-Object{'"'+$_+'"'}
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $player=Start-Process -FilePath (Resolve-Path -LiteralPath $MpvPath).Path -ArgumentList $quoted -WindowStyle Hidden -PassThru
    $ready=$false
    for($attempt=0;$attempt -lt 25;$attempt++) {
        try {if((Invoke-Mpv @('get_property','vo-configured')).data -eq $true){$ready=$true;break}}catch{}
        Start-Sleep -Milliseconds 50
    }
    if(-not $ready){throw 'Video output did not initialize during extraction delay'}
    $voTime=$clock.Elapsed.TotalSeconds
    if($voTime -ge 4){throw 'Video output initialized only after the artificial extraction delay'}
    if($null -ne (Invoke-Mpv @('get_property','time-pos')).data){throw 'Fixture already loaded; early-init assertion invalid'}
    if((Invoke-Mpv @('expand-text',$osd)).data -ne 'Loading video...'){throw 'Loading message absent during extraction delay'}
    $loaded=$false
    for($attempt=0;$attempt -lt 80;$attempt++) {
        if($null -ne (Invoke-Mpv @('get_property','time-pos')).data){$loaded=$true;break}
        Start-Sleep -Milliseconds 100
    }
    if(-not $loaded){throw 'Playback never became ready after extraction delay'}
    if((Invoke-Mpv @('expand-text',$osd)).data -ne ''){throw 'Loading message did not clear'}
    Write-Output ('Video output ready at {0:N2}s; media ready at {1:N2}s with a 4s simulated extraction delay. Loading OSD cleared.' -f $voTime,$clock.Elapsed.TotalSeconds)
}finally{
    if($player -and -not $player.HasExited){Stop-Process -Id $player.Id -Force}
    foreach($file in @($wave,$script,$log)){if(Test-Path -LiteralPath $file){Remove-Item -LiteralPath $file -Force}}
    if(Test-Path -LiteralPath $directory){Remove-Item -LiteralPath $directory}
}
