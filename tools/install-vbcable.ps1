param([Parameter(Mandatory=$true)][string]$WorkDirectory)
$ErrorActionPreference = 'Stop'
# This script is embedded only in builds whose operator has obtained integration rights.
$archive = Join-Path $WorkDirectory 'vbcable.zip'
$partial = "$archive.part"
$expected = 'B950E39F01AF1D04EA623C8F6D8EB9B6EA5C477C637295FABF20631C85116BFB'
$uri = 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip'
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -Uri $uri -OutFile $partial -TimeoutSec 120
    if ((Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ne $expected) { throw 'The official package changed or the download is damaged. No installer was executed.' }
    Move-Item -LiteralPath $partial -Destination $archive -Force
    $extracted = Join-Path $WorkDirectory 'official-driver'
    Expand-Archive -LiteralPath $archive -DestinationPath $extracted -Force
    $setup = Join-Path $extracted 'VBCABLE_Setup_x64.exe'
    $signature = Get-AuthenticodeSignature -LiteralPath $setup
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'BUREL VINCENT') { throw 'The driver installer signature could not be verified.' }
    # Official interactive setup retains the vendor's terms and reboot instructions.
    # Do not use silent flags whose behavior is not documented by the vendor.
    $process = Start-Process -FilePath $setup -Verb RunAs -Wait -PassThru -WindowStyle Normal
    if ($process.ExitCode -notin @(0, 3010)) { throw "Driver setup exited with code $($process.ExitCode). It may have been cancelled." }
    'Official setup completed. Restart Windows if requested by VB-CABLE, then reopen MicLift. Device detection will confirm availability.'
} finally {
    if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force }
}
