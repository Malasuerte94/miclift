param([string]$Iscc = '')
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $root
try {
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $version = ($metadata.packages | Where-Object name -eq 'miclift').version
    $dist = Join-Path $root 'dist'
    New-Item -ItemType Directory -Path $dist -Force | Out-Null
    if (-not $Iscc) {
        $candidates = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", (Join-Path $root 'target\tools\InnoSetup\ISCC.exe'))
        $Iscc = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        if (-not $Iscc) { $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue; if ($command) { $Iscc = $command.Source } }
    }
    if (-not $Iscc) { throw 'Install Inno Setup 6 or pass -Iscc <path-to-ISCC.exe>' }
    $certificate = $null
    try {
        if ($env:MICLIFT_CERTIFICATE_BASE64) {
            $bytes = [Convert]::FromBase64String($env:MICLIFT_CERTIFICATE_BASE64)
            $flags = [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet
            $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($bytes, $env:MICLIFT_CERTIFICATE_PASSWORD, $flags)
        }
        function Sign-Artifact([string]$Path) {
            if ($null -ne $certificate) {
                $signature = Set-AuthenticodeSignature -FilePath $Path -Certificate $certificate -HashAlgorithm SHA256 -TimestampServer 'http://timestamp.digicert.com'
                if ($signature.Status -ne 'Valid') { throw "Signing failed for $Path" }
            }
        }
        Sign-Artifact (Join-Path $root 'target\release\miclift.exe')
        Sign-Artifact (Join-Path $root 'target\release\miclift-diagnostics.exe')
        & $Iscc '/Qp' "/DAppVersion=$version" "/DAppRoot=$root" (Join-Path $root 'installer\miclift.iss')
        if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
        $installer = Join-Path $dist "MicLift-$version-windows-x64-setup.exe"
        Sign-Artifact $installer
        $stage = Join-Path $root "target\package-$version"
        New-Item -ItemType Directory -Path $stage -Force | Out-Null
        foreach ($file in @('target\release\miclift.exe','target\release\miclift-diagnostics.exe','README.md','LICENSE','THIRD_PARTY_NOTICES.md')) { Copy-Item -LiteralPath (Join-Path $root $file) -Destination $stage -Force }
        Copy-Item -LiteralPath (Join-Path $root 'licenses') -Destination $stage -Recurse -Force
        Get-ChildItem -LiteralPath $stage -File -Recurse | Where-Object LastWriteTime -LT ([datetime]'1980-01-01') | ForEach-Object { $_.LastWriteTime = [datetime]'1980-01-01' }
        $zip = Join-Path $dist "MicLift-$version-windows-x64-portable.zip"
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
        $checksumLines = @($installer,$zip) | ForEach-Object { $hash = Get-FileHash -LiteralPath $_ -Algorithm SHA256; "$($hash.Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($_))" }
        $checksumLines | Set-Content -LiteralPath (Join-Path $dist 'SHA256SUMS.txt') -Encoding ascii
        @("Version: $version", "Authenticode signed: $($null -ne $certificate)", 'VB-CABLE is not included in these artifacts.') | Set-Content -LiteralPath (Join-Path $dist 'BUILD-INFO.txt') -Encoding utf8
    } finally { if ($null -ne $certificate) { $certificate.Dispose() } }
} finally { Pop-Location }
