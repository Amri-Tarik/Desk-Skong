<#
.SYNOPSIS
    Build Desk-Skong's release binaries and pack the portable zip:
    dist\Desk-Skong-<version>-windows-x64.zip

.DESCRIPTION
    The zip holds skong.exe, skong-panel.exe, the data\ folder (metadata JSON only), the
    readme, the licence file and FONTS-OFL.txt (the licence of the two fonts embedded in the
    exes, Cinzel and Crimson Pro, SIL OFL 1.1). No game art and no game text, ever: the players' own
    Hollow Knight install provides the art on first run.

    Every file going into the zip is checked; the script fails if anything looks like game
    art (images, audio, Unity asset files, embedded PNG data) or if data\ holds anything
    but JSON tracked by git. The finished zip is checked again entry by entry.

    Works with Windows PowerShell 5.1 and PowerShell 7.

.PARAMETER OutDir
    Where the zip goes (default: dist\ in the repository; git-ignored).

.PARAMETER SkipBuild
    Reuse target\release\*.exe instead of running cargo build.

.PARAMETER SelfTest
    Check the art guard itself (a clean folder passes, folders with art fail), then exit.
    Builds nothing, writes only to a temporary folder.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\package.ps1
#>
[CmdletBinding()]
param(
    [string]$OutDir,
    [switch]$SkipBuild,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $Root 'dist' }

# Anything with these extensions is never shipped (game art, audio, Unity assets, dumps).
$Forbidden = @(
    '.png', '.jpg', '.jpeg', '.bmp', '.gif', '.tga', '.dds', '.psd', '.webp', '.ico',
    '.wav', '.ogg', '.mp3', '.flac', '.bank', '.fsb',
    '.assets', '.ress', '.resource', '.bundle', '.unity3d', '.tpk', '.dll', '.pdb',
    '.txt', '.bytes', '.csv', '.zip', '.7z'
)
# Files the zip may hold, by path inside the package folder.
$AllowedTop = @('skong.exe', 'skong-panel.exe', 'README.md', 'LICENSE.txt', 'FONTS-OFL.txt')

function Test-PackageContent {
    <#
    Returns a list of problems (empty = fine) for the files of a package folder. $Files are
    paths relative to the package root with forward slashes; $ReadFile gives a file's bytes
    (or $null to skip content checks).
    #>
    param([string[]]$Files, [scriptblock]$ReadFile)
    $problems = New-Object System.Collections.Generic.List[string]
    foreach ($f in $Files) {
        $ext = [System.IO.Path]::GetExtension($f).ToLowerInvariant()
        if ($AllowedTop -contains $f) { continue }
        if ($Forbidden -contains $ext) { $problems.Add("forbidden file type: $f"); continue }
        if ($f -notlike 'data/*') { $problems.Add("unexpected file: $f"); continue }
        if ($ext -ne '.json') { $problems.Add("data/ holds a non-JSON file: $f"); continue }
        if ($ReadFile) {
            [byte[]]$bytes = & $ReadFile $f
            if ($null -eq $bytes) { continue }
            # Raw PNG signature, or base64-encoded PNG / JPEG data inside the JSON.
            if ($bytes.Length -ge 8 -and $bytes[0] -eq 0x89 -and $bytes[1] -eq 0x50 -and $bytes[2] -eq 0x4E -and $bytes[3] -eq 0x47) {
                $problems.Add("PNG data in $f")
            } else {
                $text = [System.Text.Encoding]::UTF8.GetString($bytes)
                if ($text.Contains('iVBORw0KGgo') -or $text.Contains('/9j/4AAQ') -or $text -match 'data:image/') {
                    $problems.Add("embedded image data in $f")
                }
            }
        }
    }
    return ,$problems
}

function Get-FolderFiles([string]$Dir) {
    $base = (Resolve-Path $Dir).Path.TrimEnd('\') + '\'
    Get-ChildItem -LiteralPath $Dir -Recurse -File -Force | ForEach-Object { $_.FullName.Substring($base.Length).Replace('\', '/') }
}

function Assert-Clean([string]$Dir) {
    $files = @(Get-FolderFiles $Dir)
    $read = { param($rel) [System.IO.File]::ReadAllBytes((Join-Path $Dir $rel)) }.GetNewClosure()
    $problems = Test-PackageContent -Files $files -ReadFile $read
    if ($problems.Count -gt 0) {
        throw "package check failed (no game art may ship):`n  " + ($problems -join "`n  ")
    }
    return $files.Count
}

if ($SelfTest) {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("skong-package-selftest-" + [guid]::NewGuid().ToString('N'))
    $failures = 0
    function New-Case([string]$name, [hashtable]$files) {
        $d = Join-Path $tmp $name
        foreach ($k in $files.Keys) {
            $p = Join-Path $d $k
            New-Item -ItemType Directory -Force (Split-Path -Parent $p) | Out-Null
            [System.IO.File]::WriteAllBytes($p, [byte[]]$files[$k])
        }
        return $d
    }
    $json = [System.Text.Encoding]::UTF8.GetBytes('{"fps": 12}')
    $base = @{ 'skong.exe' = @(77, 90); 'skong-panel.exe' = @(77, 90); 'README.md' = @(35); 'LICENSE.txt' = @(84); 'FONTS-OFL.txt' = @(83); 'data/hk/animations/Knight.json' = $json }
    $cases = @(
        @{ name = 'clean'; extra = @{}; ok = $true },
        @{ name = 'png'; extra = @{ 'data/hk/Knight.png' = @(0x89, 0x50, 0x4E, 0x47, 13, 10, 26, 10) }; ok = $false },
        @{ name = 'png-anywhere'; extra = @{ 'skins/Vanilla/Knight.png' = @(1) }; ok = $false },
        @{ name = 'audio'; extra = @{ 'data/sfx.wav' = @(1) }; ok = $false },
        @{ name = 'game-text'; extra = @{ 'data/dialogue.txt' = @(65) }; ok = $false },
        @{ name = 'stray'; extra = @{ 'notes.md' = @(65) }; ok = $false },
        @{ name = 'embedded'; extra = @{ 'data/atlas.json' = [System.Text.Encoding]::UTF8.GetBytes('{"img":"iVBORw0KGgoAAAANSUhEUg"}') }; ok = $false },
        @{ name = 'disguised'; extra = @{ 'data/atlas.json' = @(0x89, 0x50, 0x4E, 0x47, 13, 10, 26, 10) }; ok = $false }
    )
    try {
        foreach ($c in $cases) {
            $all = $base.Clone()
            foreach ($k in $c.extra.Keys) { $all[$k] = $c.extra[$k] }
            $dir = New-Case $c.name $all
            $passed = $true
            $why = ''
            try { Assert-Clean $dir | Out-Null } catch { $passed = $false; $why = ($_.Exception.Message -split "`n")[-1].Trim() }
            if ($passed -ne $c.ok) {
                Write-Host "FAIL: case '$($c.name)' should have $(if ($c.ok) { 'passed' } else { 'been rejected' }) $why"
                $failures++
            } else {
                Write-Host "ok:   $($c.name) $why"
            }
        }
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
    if ($failures -gt 0) { throw "$failures self-test case(s) failed" }
    Write-Host 'package guard self-test passed'
    return
}

Push-Location $Root
try {
    # Version from Cargo (the workspace version, as the skong package reports it).
    $meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
    $version = ($meta.packages | Where-Object { $_.name -eq 'skong' }).version
    if (-not $version) { throw 'could not read the skong version from cargo metadata' }
    $name = "Desk-Skong-$version-windows-x64"
    Write-Host "packaging $name"

    if (-not $SkipBuild) {
        cargo build --release --locked -p skong -p skong-panel
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    }
    $exes = @('skong.exe', 'skong-panel.exe') | ForEach-Object { Join-Path $Root "target\release\$_" }
    foreach ($e in $exes) { if (-not (Test-Path $e)) { throw "missing $e (build first, or drop -SkipBuild)" } }

    # Data: only files git tracks (never local exports lying around), only JSON.
    $tracked = @(git -c core.quotepath=off ls-files -- data)
    if ($LASTEXITCODE -ne 0 -or $tracked.Count -eq 0) { throw 'git ls-files data failed (run from a git checkout)' }

    $stage = Join-Path $OutDir $name
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null
    foreach ($e in $exes) { Copy-Item $e $stage }
    foreach ($f in $tracked) {
        $dest = Join-Path $stage $f
        New-Item -ItemType Directory -Force (Split-Path -Parent $dest) | Out-Null
        Copy-Item (Join-Path $Root $f) $dest
    }
    Copy-Item (Join-Path $Root 'readme.md') (Join-Path $stage 'README.md')
    # The embedded fonts' licence (OFL 1.1 asks for it to travel with the fonts).
    Copy-Item (Join-Path $Root 'crates\skong-core\fonts\OFL.txt') (Join-Path $stage 'FONTS-OFL.txt')
    $licence = @('LICENSE', 'LICENSE.md', 'LICENSE.txt', 'LICENCE') | ForEach-Object { Join-Path $Root $_ } | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($licence) {
        Copy-Item $licence (Join-Path $stage 'LICENSE.txt')
    } else {
        Write-Warning 'no LICENSE file in the repository yet: shipping a placeholder (TODO for the owner)'
        Set-Content -Encoding UTF8 (Join-Path $stage 'LICENSE.txt') @(
            'Desk-Skong licence: not chosen yet (TODO for the project owner).',
            '',
            'Until a licence is published in the repository, all rights to the Desk-Skong code are reserved by its author.',
            '',
            'Desk-Skong is an unofficial fan project, not affiliated with or endorsed by Team Cherry.',
            'Hollow Knight and its characters, art and music are property of Team Cherry.',
            'No game files are included: the art is read from your own Hollow Knight install.'
        )
    }

    $count = Assert-Clean $stage
    Write-Host "staged $count files, no game art"

    $zip = Join-Path $OutDir "$name.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    # Entry by entry with forward slashes (ZipFile.CreateFromDirectory on Windows
    # PowerShell 5.1 writes backslashes, which some unzippers mangle).
    $out = [System.IO.Compression.ZipFile]::Open($zip, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($rel in @(Get-FolderFiles $stage | Sort-Object)) {
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($out, (Join-Path $stage $rel), "$name/$rel", [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    } finally {
        $out.Dispose()
    }

    # Check the zip itself, entry by entry (what players actually get).
    $archive = [System.IO.Compression.ZipFile]::OpenRead($zip)
    try {
        $prefix = "$name/"
        $entries = @{}
        foreach ($en in $archive.Entries) {
            if ($en.FullName.EndsWith('/')) { continue }
            $rel = $en.FullName
            if ($rel.Contains('\')) { throw "zip entry with a backslash: $rel" }
            if (-not $rel.StartsWith($prefix)) { throw "zip entry outside $prefix : $rel" }
            $entries[$rel.Substring($prefix.Length)] = $en
        }
        $read = {
            param($rel)
            $s = $entries[$rel].Open()
            try { $ms = New-Object System.IO.MemoryStream; $s.CopyTo($ms); , $ms.ToArray() } finally { $s.Dispose() }
        }.GetNewClosure()
        $problems = Test-PackageContent -Files @($entries.Keys) -ReadFile $read
        if ($problems.Count -gt 0) { throw "zip check failed:`n  " + ($problems -join "`n  ") }
        foreach ($must in @('skong.exe', 'skong-panel.exe', 'README.md', 'LICENSE.txt', 'FONTS-OFL.txt', 'data/achievements.json')) {
            if (-not $entries.ContainsKey($must)) { throw "zip is missing $must" }
        }
    } finally {
        $archive.Dispose()
    }
    $size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "wrote $zip ($size MB)"
    if ($env:GITHUB_OUTPUT) {
        Add-Content -Path $env:GITHUB_OUTPUT -Value "zip=$zip"
        Add-Content -Path $env:GITHUB_OUTPUT -Value "version=$version"
    }
} finally {
    Pop-Location
}
