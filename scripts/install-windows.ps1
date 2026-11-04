# EntoCAD per-user installer. Run again after git pull to update existing shortcuts.
# Requires Rust/MSVC, Visual Studio C++ build tools, and LLVM/libclang.
# Dependencies are never downloaded by this script. Compatible with PowerShell 5.1.
[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$Launch,
    [ValidateRange(0, 2147483647)][int]$WaitPid = 0,
    [string]$Reopen,
    [string]$LogPath,
    [switch]$Help
)
$ErrorActionPreference = "Stop"
if ($Help) {
    Write-Host @'
Usage: powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\install-windows.ps1 [options]
  -Launch          Start EntoCAD after a successful installation.
  -Reopen PATH     Open this absolute drawing path afterward (requires -Launch).
  -WaitPid PID     Wait up to two minutes for this process; never force-close it.
  -LogPath PATH    Append installer output to this log file.
  -Uninstall       Remove the program, shortcut, and Open with registration.
  -Help            Show this help.

Run from a checkout with Rust/MSVC, Visual Studio C++ tools, and LLVM/libclang installed.
No administrator privileges are needed. Existing taskbar pins use the updated app.
'@
    exit 0
}
$AppName = "EntoCAD"
$ProgId = "EntoCAD.Drawing"
$Extensions = @(".dwg", ".dxf")
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$InstallLock = $null
$TranscriptStarted = $false
$ExitCode = 0

function Get-WorkspaceVersion {
    $inPackage = $false
    foreach ($line in Get-Content -LiteralPath (Join-Path $RepoRoot "Cargo.toml")) {
        if ($line -match '^\[workspace\.package\]\s*$') { $inPackage = $true; continue }
        if ($inPackage -and $line -match '^\[') { break }
        if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') { return $Matches[1] }
    }
    throw "Could not read workspace.package.version from Cargo.toml."
}
function Wait-ForPreviousApp {
    if ($WaitPid -eq 0) { return }
    if ($WaitPid -eq $PID) { throw "The installer cannot wait for its own process." }
    $previous = Get-Process -Id $WaitPid -ErrorAction SilentlyContinue
    if ($null -eq $previous) { return }
    try {
        Write-Host "Waiting for EntoCAD to finish closing..."
        if (-not $previous.WaitForExit(120000)) {
            throw "EntoCAD is still running. Close it normally, then run the installer again."
        }
    }
    finally { $previous.Dispose() }
}
function Assert-AppStopped {
    foreach ($running in @(Get-Process -Name "EntoCAD" -ErrorAction SilentlyContinue)) {
        $processPath = $null
        # Other users' processes may be inaccessible and do not lock this user's copy.
        try { $processPath = $running.Path } catch { }
        if ($processPath -and [StringComparer]::OrdinalIgnoreCase.Equals($processPath, $InstalledExe)) {
            throw "EntoCAD is still running. Close it normally, then run this script again."
        }
    }
}
function Set-RegistryString {
    param([string]$Key, [string]$Name, [string]$Value)
    New-Item -Path $Key -Force | Out-Null
    New-ItemProperty -LiteralPath $Key -Name $Name -PropertyType String -Value $Value -Force | Out-Null
}
function Register-EntoCad {
    param([string]$Version)
    New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
    $temporaryShortcut = Join-Path $StartMenuDir ("EntoCAD-" + [Guid]::NewGuid().ToString("N") + ".lnk")
    try {
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($temporaryShortcut)
        $shortcut.TargetPath = $InstalledExe
        $shortcut.WorkingDirectory = $InstallDir
        $shortcut.WindowStyle = 1
        $shortcut.Description = $AppName
        $shortcut.IconLocation = "$InstalledExe,0"
        $shortcut.Save()
        Move-Item -LiteralPath $temporaryShortcut -Destination $ShortcutPath -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporaryShortcut) { Remove-Item -LiteralPath $temporaryShortcut -Force }
    }
    $openCommand = "`"$InstalledExe`" `"%1`""
    foreach ($item in @(
        @{ Key = $ProgIdKey; Value = "$AppName drawing" },
        @{ Key = "$ProgIdKey\DefaultIcon"; Value = "`"$InstalledExe`",0" },
        @{ Key = "$ProgIdKey\shell\open\command"; Value = $openCommand },
        @{ Key = "$ApplicationsKey\shell\open\command"; Value = $openCommand }
    )) {
        New-Item -Path $item.Key -Force | Out-Null
        Set-Item -LiteralPath $item.Key -Value $item.Value
    }
    Set-RegistryString $ApplicationsKey "FriendlyAppName" $AppName
    foreach ($extension in $Extensions) {
        Set-RegistryString "$ClassesRoot\$extension\OpenWithProgids" $ProgId ""
        Set-RegistryString "$ApplicationsKey\SupportedTypes" $extension ""
    }
    $uninstallCommand = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$InstalledUninstaller`" -Uninstall"
    $properties = @{
        DisplayName = $AppName; DisplayVersion = $Version; Publisher = "Entomak"
        DisplayIcon = "`"$InstalledExe`",0"; InstallLocation = $InstallDir
        UninstallString = $uninstallCommand
    }
    foreach ($name in $properties.Keys) { Set-RegistryString $UninstallKey $name $properties[$name] }
    foreach ($name in @("NoModify", "NoRepair")) {
        New-ItemProperty -LiteralPath $UninstallKey -Name $name -PropertyType DWord -Value 1 -Force | Out-Null
    }
    $sizeKb = [math]::Ceiling((Get-Item -LiteralPath $InstalledExe).Length / 1KB)
    New-ItemProperty -LiteralPath $UninstallKey -Name "EstimatedSize" -PropertyType DWord -Value $sizeKb -Force | Out-Null
}
function Uninstall-EntoCad {
    Assert-AppStopped
    # Delete only installer-owned files. Keep arbitrary files placed beside the app.
    foreach ($path in @($InstalledExe, $SourcePath, $InstalledUninstaller)) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    foreach ($extension in $Extensions) {
        foreach ($key in @(
            "$ClassesRoot\$extension\OpenWithProgids",
            "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$extension\OpenWithProgids"
        )) {
            if (Test-Path -LiteralPath $key) { Remove-ItemProperty -LiteralPath $key -Name $ProgId -ErrorAction SilentlyContinue }
        }
    }
    foreach ($key in @($ProgIdKey, $ApplicationsKey, $UninstallKey)) {
        if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force }
    }
    if (Test-Path -LiteralPath $ShortcutPath) { Remove-Item -LiteralPath $ShortcutPath -Force }
    if ((Test-Path -LiteralPath $InstallDir) -and -not (Get-ChildItem -LiteralPath $InstallDir -Force)) {
        Remove-Item -LiteralPath $InstallDir
    }
    Write-Host "EntoCAD was removed. Drawings and preferences were kept."
}
function Install-EntoCad {
    if (-not (Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue)) {
        throw "Cargo was not found. Install Rust's MSVC toolchain, Visual Studio C++ build tools, and LLVM/libclang, then retry."
    }
    Assert-AppStopped
    Write-Host "Building EntoCAD (release). The first build can take several minutes..."
    $builtExe = $null
    Push-Location $RepoRoot
    try {
        # Cargo reports the executable, including custom target directories/triples.
        & cargo build --release -p mycad --locked --message-format=json-render-diagnostics | ForEach-Object {
            $message = $null
            try { $message = $_ | ConvertFrom-Json -ErrorAction Stop } catch { Write-Host $_ }
            if ($null -ne $message) {
                if ($message.reason -eq "compiler-artifact" -and $message.target.name -eq "mycad" -and $message.executable) {
                    $builtExe = $message.executable
                }
                elseif ($message.reason -eq "compiler-message" -and $message.message.rendered) {
                    Write-Host $message.message.rendered
                }
            }
        }
        if ($LASTEXITCODE -ne 0) {
            throw "The build failed. The installed app has not changed. Check Rust/MSVC and LLVM prerequisites; for compiler memory failures retry cargo build --release -p mycad --locked -j 1."
        }
    }
    finally { Pop-Location }
    if (-not $builtExe -or -not (Test-Path -LiteralPath $builtExe -PathType Leaf)) {
        throw "Cargo did not produce the mycad executable. The installed app has not changed."
    }
    $version = Get-WorkspaceVersion
    $source = [ordered]@{
        repo = $RepoRoot; update_command = $PSCommandPath; protocol_version = 1
        installed_version = $version
    }
    if (Get-Command git -CommandType Application -ErrorAction SilentlyContinue) {
        try {
            $commit = & git -C $RepoRoot rev-parse HEAD 2>$null
            if ($LASTEXITCODE -eq 0 -and $commit -match '^[0-9a-f]{40,64}$') { $source["installed_commit"] = $commit }
        }
        catch { Write-Warning "Could not record the optional Git revision." }
    }
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $transaction = [Guid]::NewGuid().ToString("N")
    $entries = @($InstalledExe, $SourcePath, $InstalledUninstaller) | ForEach-Object {
        @{ Target = $_; Stage = "$_.${transaction}.new"; Backup = "$_.${transaction}.backup"; Applied = $false }
    }
    $keepBackups = $false
    try {
        Copy-Item -LiteralPath $builtExe -Destination $entries[0].Stage
        [IO.File]::WriteAllText($entries[1].Stage, ($source | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
        Copy-Item -LiteralPath $PSCommandPath -Destination $entries[2].Stage
        Assert-AppStopped
        foreach ($entry in $entries) {
            if (Test-Path -LiteralPath $entry.Target) { [IO.File]::Replace($entry.Stage, $entry.Target, $entry.Backup, $true) }
            else { [IO.File]::Move($entry.Stage, $entry.Target) }
            $entry.Applied = $true
        }
        Register-EntoCad $version
    }
    catch {
        $installError = $_
        for ($i = $entries.Count - 1; $i -ge 0; $i--) {
            $entry = $entries[$i]
            if (-not $entry.Applied) { continue }
            try {
                if (Test-Path -LiteralPath $entry.Backup) {
                    # A concrete rollback destination avoids PowerShell coercing
                    # a null backup argument to an invalid empty string.
                    [IO.File]::Replace($entry.Backup, $entry.Target, $entry.Stage, $true)
                }
                else { Remove-Item -LiteralPath $entry.Target -Force }
            }
            catch {
                $keepBackups = $true
                Write-Warning "Rollback needs attention. Backup retained at $($entry.Backup): $($_.Exception.Message)"
            }
        }
        throw "Installation failed. Existing program files were restored where possible. Close EntoCAD and retry. $($installError.Exception.Message)"
    }
    finally {
        foreach ($entry in $entries) {
            if (Test-Path -LiteralPath $entry.Stage) { Remove-Item -LiteralPath $entry.Stage -Force -ErrorAction SilentlyContinue }
            if (-not $keepBackups -and (Test-Path -LiteralPath $entry.Backup)) {
                Remove-Item -LiteralPath $entry.Backup -Force -ErrorAction SilentlyContinue
            }
        }
    }
    Write-Host "EntoCAD $version is installed at $InstalledExe"
    Write-Host "Start menu: $ShortcutPath"
    Write-Host "Pin EntoCAD from the Start menu to keep it on your taskbar. Existing pins open this updated copy."
    Write-Host "DWG and DXF are registered under Open with; your existing default apps are unchanged."
}
function ConvertTo-WindowsArgument {
    param([string]$Value)
    # ProcessStartInfo.Arguments uses Windows CRT quoting, not shell quoting.
    $result = [Text.StringBuilder]::new()
    [void]$result.Append('"')
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char]92) { $slashes++; continue }
        if ($character -eq '"') { [void]$result.Append([char]92, (2 * $slashes + 1)) }
        elseif ($slashes -gt 0) { [void]$result.Append([char]92, $slashes) }
        [void]$result.Append($character)
        $slashes = 0
    }
    if ($slashes -gt 0) { [void]$result.Append([char]92, (2 * $slashes)) }
    [void]$result.Append('"')
    return $result.ToString()
}
try {
    if ($env:OS -ne "Windows_NT" -or -not $env:LOCALAPPDATA -or -not $env:APPDATA) {
        throw "This installer runs on Windows. Use install-linux.sh or install-macos.sh for other systems."
    }
    if ($Uninstall -and ($Launch -or $Reopen)) { throw "-Uninstall cannot be combined with -Launch or -Reopen." }
    if ($Reopen -and (-not $Launch -or $Reopen -notmatch '^(?:[A-Za-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+)')) {
        throw "-Reopen requires -Launch and an absolute drawing path."
    }
    $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\EntoCAD"
    $InstalledExe = Join-Path $InstallDir "EntoCAD.exe"
    $SourcePath = Join-Path $InstallDir "EntoCAD.source.json"
    $InstalledUninstaller = Join-Path $InstallDir "uninstall.ps1"
    $StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $ShortcutPath = Join-Path $StartMenuDir "EntoCAD.lnk"
    $UninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\EntoCAD"
    $ClassesRoot = "HKCU:\Software\Classes"
    $ProgIdKey = "$ClassesRoot\$ProgId"
    $ApplicationsKey = "$ClassesRoot\Applications\EntoCAD.exe"
    $LogDir = Join-Path $env:LOCALAPPDATA "EntoCAD\logs"
    New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
    try {
        $InstallLock = [IO.File]::Open((Join-Path $LogDir "install-windows.lock"), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    }
    catch { throw "Another EntoCAD installer may already be running, or the log directory is not writable. $($_.Exception.Message)" }
    if (-not $LogPath) { $LogPath = Join-Path $LogDir "install-windows.log" }
    $LogPath = [IO.Path]::GetFullPath($LogPath)
    New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($LogPath)) | Out-Null
    Start-Transcript -LiteralPath $LogPath -Append | Out-Null
    $TranscriptStarted = $true
    Write-Host "Installer log: $LogPath"
    Wait-ForPreviousApp
    if ($Uninstall) { Uninstall-EntoCad }
    else {
        Install-EntoCad
        if ($Launch) {
            $start = [Diagnostics.ProcessStartInfo]::new()
            $start.FileName = $InstalledExe
            $start.WorkingDirectory = $InstallDir
            $start.UseShellExecute = $false
            if ($Reopen) { $start.Arguments = ConvertTo-WindowsArgument $Reopen }
            $started = [Diagnostics.Process]::Start($start)
            $started.Dispose()
        }
    }
}
catch {
    $ExitCode = 1
    $failure = "EntoCAD installer failed: $($_.Exception.Message)"
    if ($LogPath) { $failure += "`r`n`r`nInstaller log: $LogPath" }
    Write-Host $failure -ForegroundColor Red
    if ($Launch) {
        try {
            Add-Type -AssemblyName System.Windows.Forms
            [void][Windows.Forms.MessageBox]::Show($failure, "EntoCAD update failed", [Windows.Forms.MessageBoxButtons]::OK, [Windows.Forms.MessageBoxIcon]::Error)
        }
        catch { Write-Warning "Could not display the error dialog. See the installer output above." }
    }
}
finally {
    try { if ($TranscriptStarted) { Stop-Transcript | Out-Null } }
    finally { if ($null -ne $InstallLock) { $InstallLock.Dispose() } }
}
exit $ExitCode
