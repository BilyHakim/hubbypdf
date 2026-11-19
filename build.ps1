param([switch]$Test, [string]$OutputDirectory = 'target/HubbyPDF')

$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    $archive = 'pdfium-win-x64.tgz'
    $vendor = 'vendor/pdfium'
    $hash = '0B08B606792A6CC593426EFDEFC6622611BCE446D9E0270743846956EA1554CA'
    if (!(Test-Path -LiteralPath "$vendor/bin/pdfium.dll")) {
        if (!(Test-Path -LiteralPath $archive)) {
            $url = 'https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F7543/pdfium-win-x64.tgz'
            Invoke-WebRequest -UseBasicParsing $url -OutFile $archive
        }
        if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $hash) {
            throw "Checksum PDFium berbeda. Hapus $archive dan jalankan ulang."
        }
        New-Item -ItemType Directory -Force $vendor | Out-Null
        tar -xzf $archive -C $vendor
        if ($LASTEXITCODE -ne 0) { throw 'Ekstraksi PDFium gagal.' }
    }

    $target = 'x86_64-pc-windows-msvc'
    if ($Test) {
        cargo test --locked --target $target
        if ($LASTEXITCODE -ne 0) { throw 'Pengujian gagal.' }
    }
    cargo build --release --locked --target $target
    if ($LASTEXITCODE -ne 0) { throw 'Build HubbyPDF gagal.' }

    $dist = $OutputDirectory
    New-Item -ItemType Directory -Force $dist | Out-Null
    Copy-Item -LiteralPath "target/$target/release/hubbypdf.exe" -Destination "$dist/HubbyPDF.exe" -Force
    Copy-Item -LiteralPath "$vendor/bin/pdfium.dll" -Destination $dist -Force
    Copy-Item -LiteralPath "$vendor/LICENSE" -Destination "$dist/PDFium-LICENSE.txt" -Force
    Copy-Item -LiteralPath "$vendor/licenses" -Destination $dist -Recurse -Force
    Write-Output (Join-Path (Resolve-Path $dist) 'HubbyPDF.exe')
} finally {
    Pop-Location
}

