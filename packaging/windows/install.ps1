# EntoCAD per-user install for Windows.
#
# Builds the release binary, copies it out of the repo, and adds a Start
# menu shortcut, an Open with entry for DWG and DXF, and a Settings > Apps
# entry. Re-run this script after a git pull to update the installed copy.
#
#   powershell -ExecutionPolicy Bypass -File packaging\windows\install.ps1
#   powershell -ExecutionPolicy Bypass -File packaging\windows\install.ps1 -Uninstall

[CmdletBinding()]
param(
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

# Names and paths live here so the rest of the script does not repeat them.
$AppName = "EntoCAD"
$Publisher = "Entomak"
$ProgId = "EntoCAD.Drawing"
$Extensions = @(".dwg", ".dxf")
$ExeFileName = "EntoCAD.exe"
$BuiltExeName = "mycad.exe"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\$AppName"
$InstalledExe = Join-Path $InstallDir $ExeFileName
$StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$ShortcutPath = Join-Path $StartMenuDir "$AppName.lnk"
$UninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$AppName"
$ClassesRoot = "HKCU:\Software\Classes"
$ProgIdKey = Join-Path $ClassesRoot $ProgId
$ApplicationsKey = Join-Path $ClassesRoot "Applications\$ExeFileName"

function Get-WorkspaceVersion {
    $cargoToml = Join-Path $RepoRoot "Cargo.toml"
    $inPackage = $false
    foreach ($line in Get-Content -LiteralPath $cargoToml) {
        if ($line -match '^\[workspace\.package\]') {
            $inPackage = $true
            continue
        }
        if ($inPackage -and $line -match '^\[') {
            break
        }
        if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    throw "Could not read workspace.package.version from $cargoToml"
}

function Remove-OpenWithProgId {
    param([string]$Extension)

    $classKey = Join-Path $ClassesRoot "$Extension\OpenWithProgids"
    if (Test-Path -LiteralPath $classKey) {
        Remove-ItemProperty -LiteralPath $classKey -Name $ProgId -ErrorAction SilentlyContinue
    }

    $fileExts = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$Extension\OpenWithProgids"
    if (Test-Path -LiteralPath $fileExts) {
        Remove-ItemProperty -LiteralPath $fileExts -Name $ProgId -ErrorAction SilentlyContinue
    }
}

function Uninstall-EntoCad {
    foreach ($extension in $Extensions) {
        Remove-OpenWithProgId -Extension $extension
    }
    foreach ($key in @($ProgIdKey, $ApplicationsKey, $UninstallKey)) {
        if (Test-Path -LiteralPath $key) {
            Remove-Item -LiteralPath $key -Recurse -Force
        }
    }
    if (Test-Path -LiteralPath $ShortcutPath) {
        Remove-Item -LiteralPath $ShortcutPath -Force
    }
    if (Test-Path -LiteralPath $InstallDir) {
        Remove-Item -LiteralPath $InstallDir -Recurse -Force
    }
    Write-Host "$AppName was removed. Saved drawings and preferences were left in place."
}

if ($Uninstall) {
    Uninstall-EntoCad
    exit 0
}

Write-Host "Building $AppName (release)..."
Push-Location $RepoRoot
try {
    & cargo build --release -p mycad
    if ($LASTEXITCODE -ne 0) {
        throw "The build failed. If the compiler crashed, retry with: cargo build --release -p mycad -j 1"
    }
}
finally {
    Pop-Location
}

$builtExe = Join-Path $RepoRoot "target\release\$BuiltExeName"
if (-not (Test-Path -LiteralPath $builtExe)) {
    throw "Build finished but $builtExe is missing."
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
try {
    Copy-Item -LiteralPath $builtExe -Destination $InstalledExe -Force -ErrorAction Stop
}
catch {
    throw "Could not copy $ExeFileName. Close $AppName if it is running, then run this script again. $($_.Exception.Message)"
}

New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($ShortcutPath)
$shortcut.TargetPath = $InstalledExe
$shortcut.WorkingDirectory = $InstallDir
$shortcut.WindowStyle = 1
$shortcut.Description = $AppName
$shortcut.IconLocation = "$InstalledExe,0"
$shortcut.Save()

$openCommand = "`"$InstalledExe`" `"%1`""
New-Item -Path $ProgIdKey -Force | Out-Null
Set-Item -Path $ProgIdKey -Value "$AppName drawing"
$iconKey = Join-Path $ProgIdKey "DefaultIcon"
New-Item -Path $iconKey -Force | Out-Null
Set-Item -Path $iconKey -Value "$InstalledExe,0"
$commandKey = Join-Path $ProgIdKey "shell\open\command"
New-Item -Path $commandKey -Force | Out-Null
Set-Item -Path $commandKey -Value $openCommand

New-Item -Path $ApplicationsKey -Force | Out-Null
New-ItemProperty -LiteralPath $ApplicationsKey -Name "FriendlyAppName" -PropertyType String -Value $AppName -Force | Out-Null
$appCommandKey = Join-Path $ApplicationsKey "shell\open\command"
New-Item -Path $appCommandKey -Force | Out-Null
Set-Item -Path $appCommandKey -Value $openCommand

foreach ($extension in $Extensions) {
    $openWith = Join-Path $ClassesRoot "$extension\OpenWithProgids"
    New-Item -Path $openWith -Force | Out-Null
    New-ItemProperty -LiteralPath $openWith -Name $ProgId -PropertyType String -Value "" -Force | Out-Null

    $supported = Join-Path $ApplicationsKey "SupportedTypes\$extension"
    New-Item -Path $supported -Force | Out-Null
}

$version = Get-WorkspaceVersion
$sizeKb = [math]::Ceiling((Get-Item -LiteralPath $InstalledExe).Length / 1KB)
$uninstallCommand = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Uninstall"
New-Item -Path $UninstallKey -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "DisplayName" -PropertyType String -Value $AppName -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "DisplayVersion" -PropertyType String -Value $version -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "Publisher" -PropertyType String -Value $Publisher -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "DisplayIcon" -PropertyType String -Value "$InstalledExe,0" -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "InstallLocation" -PropertyType String -Value $InstallDir -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "UninstallString" -PropertyType String -Value $uninstallCommand -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "NoModify" -PropertyType DWord -Value 1 -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "NoRepair" -PropertyType DWord -Value 1 -Force | Out-Null
New-ItemProperty -LiteralPath $UninstallKey -Name "EstimatedSize" -PropertyType DWord -Value $sizeKb -Force | Out-Null

Write-Host ""
Write-Host "$AppName $version is installed."
Write-Host "Start menu: $ShortcutPath"
Write-Host "Pin it from the Start menu (right-click, Pin to taskbar)."
Write-Host "DWG and DXF show EntoCAD under Open with. Choose Always once if you want it as the default."
Write-Host "Update later by running this script again. Uninstall with -Uninstall."
