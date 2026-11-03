# Produces the pinned Windows Herdr and prints the path of its herdr.exe.
#
# The Windows counterpart of fetch-herdr-runtime.sh, which is zsh and the
# Windows runner has none. It reads everything from contracts/herdr-bundle.json
# (repository, tag, digest, version), so the pin stays in one place; the
# asset is a zip holding herdr.exe and its ConPTY runtime.
#
# The zip is kept in a cache keyed by its digest and checked again before it
# is reused; a file that no longer matches the pin is discarded, not trusted.
# Usage: pwsh -NoProfile -File scripts/fetch-herdr-runtime.ps1
$ErrorActionPreference = 'Stop'
# The path printed below is read by its callers as UTF-8 (package.mjs, the
# workflows' bash). PowerShell writes a redirected stdout in the console's
# code page, which on a Korean Windows is not UTF-8, so a user folder named
# in Korean reached package.mjs garbled (with-hide/hide#2042).
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

$root = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content -Raw -LiteralPath (Join-Path $root 'contracts/herdr-bundle.json') | ConvertFrom-Json
$pin = $manifest.windows_x86_64
if (-not $pin) { throw 'the manifest does not record a Windows asset' }

$cacheRoot = if ($env:HIDE_HERDR_CACHE) { $env:HIDE_HERDR_CACHE } else { Join-Path $env:LOCALAPPDATA 'hide\herdr-runtime' }
$folder = Join-Path $cacheRoot $pin.sha256
$zip = Join-Path $folder 'herdr.zip'
$unpacked = Join-Path $folder 'unpacked'
$exe = Join-Path $unpacked 'herdr.exe'

function Test-Digest($path) {
    (Test-Path -LiteralPath $path) -and ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash -eq $pin.sha256)
}

if (-not (Test-Digest $zip) -or -not (Test-Path -LiteralPath $exe)) {
    Remove-Item -Recurse -Force -LiteralPath $folder -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Invoke-WebRequest -Uri $pin.source_url -OutFile $zip
    if (-not (Test-Digest $zip)) {
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash
        Remove-Item -Recurse -Force -LiteralPath $folder
        throw "downloaded Herdr asset digest does not match the pin: pinned=$($pin.sha256) downloaded=$actual url=$($pin.source_url)"
    }
    Expand-Archive -LiteralPath $zip -DestinationPath $unpacked -Force
}

$reported = (& $exe --version).Trim().Split(' ')[-1]
if ($reported -ne $manifest.version) {
    throw "downloaded Herdr asset reports $reported, not the pinned $($manifest.version)"
}
Write-Output $exe
