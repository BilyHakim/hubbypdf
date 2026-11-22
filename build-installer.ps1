param(
    [switch]$Test,
    [string]$WixBin = 'target/wix314'
)

$ErrorActionPreference = 'Stop'

function Invoke-BuildTool($Tool, [string[]]$Arguments) {
    & $Tool @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Build installer gagal ($Tool, exit $LASTEXITCODE)." }
}

Push-Location $PSScriptRoot
try {
    $wix = (Resolve-Path -LiteralPath $WixBin).Path
    $candle = Join-Path $wix 'candle.exe'
    $light = Join-Path $wix 'light.exe'
    $heat = Join-Path $wix 'heat.exe'
    foreach ($tool in @($candle, $light, $heat)) {
        if (!(Test-Path -LiteralPath $tool -PathType Leaf)) { throw "Compiler WiX 3 tidak ditemukan ($tool)." }
    }

    $resourceCompiler = (Get-Command rc.exe -ErrorAction SilentlyContinue).Source
    if (!$resourceCompiler) {
        $programFiles = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
        $sdk = Join-Path $programFiles 'Windows Kits/10/bin'
        $resourceCompiler = Get-ChildItem -LiteralPath $sdk -Directory |
            Where-Object Name -Match '^\d+(\.\d+){3}$' |
            Sort-Object { [version]$_.Name } -Descending |
            ForEach-Object { Join-Path $_.FullName 'x64/rc.exe' } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
            Select-Object -First 1
    }
    if (!$resourceCompiler) { throw 'Resource compiler Windows SDK (rc.exe) tidak ditemukan.' }

    $output = (New-Item -ItemType Directory -Force 'target/installer').FullName
    $work = (New-Item -ItemType Directory -Force 'target/installer-build').FullName
    $payload = Join-Path $work 'payload'
    $icon = Join-Path $work 'HubbyPDF.ico'
    $resourceScript = Join-Path $work 'HubbyPDF.rc'
    $resource = Join-Path $work 'HubbyPDF.res'
    & ./installer/New-AppIcon.ps1 -Source 'assets/hubbypdf.png' -Destination $icon
    Set-Content -LiteralPath $resourceScript -Encoding Ascii -Value '1 ICON "HubbyPDF.ico"'
    Push-Location $work
    try {
        Invoke-BuildTool $resourceCompiler @('/nologo', '/fo', $resource, $resourceScript)
    } finally {
        Pop-Location
    }

    $oldIconResource = $env:HUBBYPDF_ICON_RES
    $oldRustFlags = $env:RUSTFLAGS
    $oldEncodedRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
    try {
        $env:HUBBYPDF_ICON_RES = $resource
        if ($oldEncodedRustFlags) {
            $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedRustFlags + [char]31 + '-Ctarget-feature=+crt-static'
        } else {
            $env:RUSTFLAGS = "$oldRustFlags -Ctarget-feature=+crt-static".Trim()
        }
        & ./build.ps1 -Test:$Test -OutputDirectory $payload
    } finally {
        $env:HUBBYPDF_ICON_RES = $oldIconResource
        $env:RUSTFLAGS = $oldRustFlags
        $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedRustFlags
    }

    $version = ((Select-String -LiteralPath Cargo.toml -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)"' |
        Select-Object -First 1).Matches.Groups[1]).Value
    if (!$version) { throw 'Versi MSI harus memiliki tiga angka di Cargo.toml.' }
    $msiVersion = [version]$version
    if ($msiVersion.Major -gt 255 -or $msiVersion.Minor -gt 255 -or $msiVersion.Build -gt 65535) {
        throw 'Versi MSI melebihi batas 255.255.65535.'
    }
    # Keep the product code stable on rebuilds and distinct for each release.
    $productCode = '2D8E53A5-4D55-4C0D-BF0E-485A{0:X2}{1:X2}{2:X4}' -f
        $msiVersion.Major, $msiVersion.Minor, $msiVersion.Build
    $licenses = (Resolve-Path -LiteralPath 'vendor/pdfium/licenses').Path
    $licenseSource = Join-Path $work 'PdfiumLicenses.wxs'
    $objectDirectory = $work + '\'
    Invoke-BuildTool $heat @('dir', $licenses, '-nologo', '-ag', '-srd', '-sfrag', '-scom', '-sreg',
        '-dr', 'LICENSEFOLDER', '-cg', 'PdfiumLicenses', '-var', 'var.LicenseSource', '-out', $licenseSource)
    Invoke-BuildTool $candle @('-nologo', '-arch', 'x64', "-dAppVersion=$version", "-dProductCode=$productCode", "-dAppSource=$payload",
        "-dLicenseSource=$licenses", "-dIconSource=$icon", '-out', $objectDirectory,
        'installer/HubbyPDF.wxs', $licenseSource)

    $msi = Join-Path $output "HubbyPDF-$version-x64.msi"
    Invoke-BuildTool $light @('-nologo', '-ext', 'WixUIExtension', '-cultures:en-us', '-out', $msi,
        (Join-Path $work 'HubbyPDF.wixobj'), (Join-Path $work 'PdfiumLicenses.wixobj'))
    Write-Output $msi
} finally {
    Pop-Location
}
