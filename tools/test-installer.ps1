$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$setup = Get-ChildItem -LiteralPath (Join-Path $root 'dist') -Filter '*-setup.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw 'No installer artifact found' }
$destination = Join-Path $root 'target\installer-smoke'
$arguments = @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/NOICONS',('/DIR="'+$destination+'"'))
$startupKey = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Windows\CurrentVersion\Run')
$startupExisted = $startupKey.GetValueNames() -contains 'MicLift'
$startupValue = if ($startupExisted) { $startupKey.GetValue('MicLift',$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { $null }
$startupKind = if ($startupExisted) { $startupKey.GetValueKind('MicLift') } else { $null }
try {
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
} finally {
    if ($startupExisted) { $startupKey.SetValue('MicLift',$startupValue,$startupKind) }
    else { $startupKey.DeleteValue('MicLift',$false) }
    $startupKey.Dispose()
}
