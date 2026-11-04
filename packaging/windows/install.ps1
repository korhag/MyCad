# Compatibility entry point for existing checkout instructions and installations.
[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$Launch,
    [ValidateRange(0, 2147483647)][int]$WaitPid = 0,
    [string]$Reopen,
    [string]$LogPath,
    [switch]$Help
)
& (Join-Path $PSScriptRoot "..\..\scripts\install-windows.ps1") @PSBoundParameters
exit $LASTEXITCODE
