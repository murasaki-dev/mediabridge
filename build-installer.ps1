$ErrorActionPreference = 'Stop'

$project = Split-Path -Parent $MyInvocation.MyCommand.Path
$stage = Join-Path $project 'dist\package'
$dist = Join-Path $project 'dist'
$exe = Join-Path $project 'target\release\mediabridge.exe'

Write-Host 'Building MediaBridge from the current workspace before packaging...'
$buildScript = Join-Path $project 'install-deps.bat'
if (-not (Test-Path -LiteralPath $buildScript)) { throw "Build script not found at $buildScript." }
$buildCommand = 'call "' + $buildScript + '"'
& $env:ComSpec /d /c $buildCommand
if ($LASTEXITCODE -ne 0) { throw "MediaBridge build failed (exit $LASTEXITCODE). Installer packaging was stopped." }
if (-not (Test-Path -LiteralPath $exe)) { throw "Build completed but the executable was not found at $exe." }

$cargoManifest = Get-Content -LiteralPath (Join-Path $project 'Cargo.toml') -Raw
$versionMatch = [regex]::Match($cargoManifest, '(?m)^version\s*=\s*"([^"\r\n]+)"')
if (-not $versionMatch.Success) { throw 'Could not read the application version from Cargo.toml.' }
$appVersion = $versionMatch.Groups[1].Value

function Find-ToolPath([string]$name, [string[]]$paths) {
    foreach ($path in $paths) { if (Test-Path -LiteralPath $path) { return $path } }
    $found = Get-Command $name -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    if ($name -eq 'ISCC.exe') {
        $uninstallRoots = @(
            'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
            'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
            'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
        )
        foreach ($root in $uninstallRoots) {
            foreach ($entry in (Get-ItemProperty $root -ErrorAction SilentlyContinue)) {
                if ($entry.DisplayName -like 'Inno Setup *' -and $entry.InstallLocation) {
                    $candidate = Join-Path $entry.InstallLocation 'ISCC.exe'
                    if (Test-Path -LiteralPath $candidate) { return $candidate }
                }
            }
        }
    }
    return $null
}

function Get-OrInstallTool([string]$name, [string]$wingetId, [string[]]$paths) {
    $existing = Find-ToolPath $name $paths
    if ($existing) { return $existing }
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw "$name is required. Install it, then rerun build-installer.bat."
    }
    Write-Host "Installing packaging tool $name..."
    & winget install --id $wingetId --exact --source winget --accept-source-agreements --accept-package-agreements
    $wingetExitCode = $LASTEXITCODE
    $installed = Find-ToolPath $name $paths
    if ($installed) { return $installed }
    if ($wingetExitCode -ne 0) {
        if ($wingetExitCode -eq -1978335189) {
            throw "winget reported that no update applies to $name, but its compiler was not found in PATH or the registered install location. Repair or install Inno Setup, then rerun build-installer.bat. Official install command: winget install --id JRSoftware.InnoSetup -e -s winget -i"
        }
        throw "winget failed to install $name (exit $wingetExitCode). Install it manually, then rerun build-installer.bat."
    }
    throw "$name was installed but its executable was not found."
}

$sevenZip = Get-OrInstallTool '7z.exe' '7zip.7zip' @(
    (Join-Path $env:ProgramFiles '7-Zip\7z.exe'),
    (Join-Path ${env:ProgramFiles(x86)} '7-Zip\7z.exe')
)
$iscc = Get-OrInstallTool 'ISCC.exe' 'JRSoftware.InnoSetup' @(
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 7\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 7\ISCC.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 7\ISCC.exe')
)

if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'mediabridge.exe')

$headers = @{ 'User-Agent' = 'MediaBridge-Windows-Installer' }
$mpvRelease = Invoke-RestMethod -Headers $headers -Uri 'https://api.github.com/repos/zhongfly/mpv-winbuild/releases/latest'
$mpvAsset = $mpvRelease.assets | Where-Object { $_.name -match '^mpv-x86_64-\d{8}-git-.+\.7z$' } | Select-Object -First 1
if (-not $mpvAsset) { throw 'Could not find a standard x86_64 mpv archive in the latest release.' }
$mpvArchive = Join-Path $dist $mpvAsset.name
New-Item -ItemType Directory -Path $dist -Force | Out-Null
Write-Host "Downloading mpv $($mpvAsset.name)..."
Invoke-WebRequest -Headers $headers -Uri $mpvAsset.browser_download_url -OutFile $mpvArchive
$mpvExtract = Join-Path $stage 'mpv-extract'
New-Item -ItemType Directory -Path $mpvExtract -Force | Out-Null
& $sevenZip x $mpvArchive "-o$mpvExtract" -y | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'Could not extract the mpv archive.' }
$mpvExe = Get-ChildItem -LiteralPath $mpvExtract -Filter 'mpv.exe' -File -Recurse | Select-Object -First 1
if (-not $mpvExe) { throw 'The downloaded mpv archive did not contain mpv.exe.' }
Get-ChildItem -LiteralPath $mpvExe.Directory.FullName -Force | Copy-Item -Destination $stage -Recurse -Force
Remove-Item -LiteralPath $mpvExtract -Recurse -Force

Write-Host 'Downloading yt-dlp...'
Invoke-WebRequest -Headers $headers -Uri 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe' -OutFile (Join-Path $stage 'yt-dlp.exe')

Write-Host 'Downloading Deno...'
$denoZip = Join-Path $dist 'deno-x86_64-pc-windows-msvc.zip'
Invoke-WebRequest -Headers $headers -Uri 'https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip' -OutFile $denoZip
$denoExtract = Join-Path $dist 'deno-extract'
if (Test-Path -LiteralPath $denoExtract) { Remove-Item -LiteralPath $denoExtract -Recurse -Force }
Expand-Archive -LiteralPath $denoZip -DestinationPath $denoExtract -Force
$denoExe = Get-ChildItem -LiteralPath $denoExtract -Filter 'deno.exe' -File -Recurse | Select-Object -First 1
if (-not $denoExe) { throw 'The downloaded Deno archive did not contain deno.exe.' }
Copy-Item -LiteralPath $denoExe.FullName -Destination (Join-Path $stage 'deno.exe')
Remove-Item -LiteralPath $denoExtract -Recurse -Force

Write-Host 'Compiling the self-contained setup executable...'
Push-Location $project
try { & $iscc "/DAppVersion=$appVersion" 'MediaBridge.iss'; if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed (exit $LASTEXITCODE)." } }
finally { Pop-Location }
Write-Host "Installer created: $(Join-Path $dist 'MediaBridge-Setup.exe')"
