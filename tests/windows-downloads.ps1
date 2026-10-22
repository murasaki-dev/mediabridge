$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $project 'build-installer.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
foreach ($name in @('Invoke-NetworkRetry', 'Save-Download')) {
    $definition = $ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name}, $true)
    Invoke-Expression $definition.Extent.Text
}
$fixture = Join-Path $project ('dist\download-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
$headers = @{}
$script:attempts = 0
$script:delays = @()
function Start-Sleep { param($Seconds) $script:delays += $Seconds }
function Invoke-WebRequest {
    param([switch]$UseBasicParsing, $Headers, $Uri, $OutFile, $TimeoutSec)
    $script:attempts++
    if ($script:attempts -eq 1) {
        [IO.File]::WriteAllText($OutFile, 'partial')
        throw 'Connection closed unexpectedly'
    }
    if (Test-Path -LiteralPath $OutFile) { throw 'Partial download was not removed before retry' }
    [IO.File]::WriteAllText($OutFile, 'complete')
}
try {
    $target = Join-Path $fixture 'tool.exe'
    Save-Download -Uri 'https://example.test/tool.exe' -OutFile $target
    if ([IO.File]::ReadAllText($target) -ne 'complete') { throw 'Incorrect final download' }
    if ($script:attempts -ne 2 -or $script:delays[0] -ne 2) { throw 'Retry behaviour incorrect' }
    if (Test-Path -LiteralPath "$target.partial") { throw 'Partial file left behind' }
    $script:failures = 0
    try {
        Invoke-NetworkRetry -Description 'fixture' -Operation { $script:failures++; throw 'offline' }
        throw 'Retry exhaustion was ignored'
    } catch {
        if ($_.Exception.Message -notlike '*failed after 4 attempts*' -or $script:failures -ne 4) { throw }
    }
    Write-Host 'Windows download retries, partial-file cleanup and retry exhaustion passed.'
} finally {
    $resolvedFixture = [IO.Path]::GetFullPath($fixture)
    $distPrefix = [IO.Path]::GetFullPath((Join-Path $project 'dist')).TrimEnd('\') + '\'
    if (-not $resolvedFixture.StartsWith($distPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe fixture path' }
    Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
}
