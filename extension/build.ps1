# Builds the browser extension from src/ into dist/:
#   dist/chrome/              -> Opera GX / Chrome: "Carregar sem compactação"
#   dist/firefox/             -> Firefox: about:debugging -> "Carregar extensão temporária"
#   dist/pincers-firefox.xpi  -> Firefox package (for signing on addons.mozilla.org)
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$dist = Join-Path $root "dist"
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }

foreach ($browser in "chrome", "firefox") {
    $out = Join-Path $dist $browser
    Copy-Item -Recurse (Join-Path $root "src") $out
    Copy-Item (Join-Path $root "manifest.$browser.json") (Join-Path $out "manifest.json")
}

# Zip with forward slashes: Windows PowerShell's Compress-Archive writes "\",
# which Firefox rejects for files in sub-folders.
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$src = Join-Path $dist "firefox"
$xpi = Join-Path $dist "pincers-firefox.xpi"
$zip = [System.IO.Compression.ZipFile]::Open($xpi, "Create")
try {
    Get-ChildItem -Recurse -File $src | ForEach-Object {
        $name = $_.FullName.Substring($src.Length + 1).Replace("\", "/")
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $_.FullName, $name) | Out-Null
    }
} finally { $zip.Dispose() }

Write-Host "Pronto:"
Write-Host "  Opera GX / Chrome: $dist\chrome"
Write-Host "  Firefox:           $xpi"
