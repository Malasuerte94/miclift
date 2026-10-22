$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
# A distinct test AppId/mutex avoids replacing the user's installed application's
# uninstall registration or closing an active MicLift session on a developer PC.
$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", (Join-Path $root 'target\tools\InnoSetup\ISCC.exe')) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $iscc) { $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue; if ($command) { $iscc = $command.Source } }
if (-not $iscc) { throw 'Inno Setup 6 is required for installer smoke checks' }
$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
$version = ($metadata.packages | Where-Object name -eq 'miclift').version
$smokeOutput = Join-Path $root 'target\installer-smoke-package'
New-Item -ItemType Directory -Path $smokeOutput -Force | Out-Null
& $iscc '/Qp' '/DSmokeTest' "/DAppVersion=$version" "/DAppRoot=$root" "/O$smokeOutput" '/Fmiclift-smoke-setup' (Join-Path $root 'installer\miclift.iss')
if ($LASTEXITCODE -ne 0) { throw 'Smoke installer compilation failed' }
$setup = Get-Item -LiteralPath (Join-Path $smokeOutput 'miclift-smoke-setup.exe')
$destination = Join-Path $root 'target\installer-smoke'
$arguments = @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/NOICONS',('/DIR="'+$destination+'"'))
# Test clean install and upgrade into a dedicated test directory; never install a driver.
foreach ($iteration in 1..2) {
    $process = Start-Process -FilePath $setup.FullName -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer pass $iteration failed: $($process.ExitCode)" }
    $app = Start-Process -FilePath (Join-Path $destination 'miclift.exe') -ArgumentList '--self-test' -WindowStyle Hidden -Wait -PassThru
    if ($app.ExitCode -ne 0) { throw 'Installed app self-test failed' }
}
$uninstaller = Join-Path $destination 'unins000.exe'
$process = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) { throw 'Uninstall failed' }
if (Test-Path -LiteralPath (Join-Path $destination 'miclift.exe')) { throw 'Uninstall left the app binary behind' }
Write-Output 'Installer install, upgrade, self-test and uninstall passed.'
