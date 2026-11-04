# Run with: pwsh -NoLogo -NoProfile -File scripts/tests/test_install_windows.ps1
# Tests installer logic in an isolated temporary directory. Registry, COM shortcuts,
# and actual Windows executable locking still require native Windows validation.
$ErrorActionPreference = 'Stop'
$scriptPath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../install-windows.ps1'))
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$legacyPath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../packaging/windows/install.ps1'))
$null = [Management.Automation.Language.Parser]::ParseFile($legacyPath, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
# Extract functions without running installer setup or touching the user installation.
# Preserve the script path normally supplied by PowerShell inside those functions.
foreach ($definition in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text.Replace('$PSCommandPath', '$scriptPath')))
}
function Assert-Equal($actual, $expected, $name) {
    if ($actual -cne $expected) { throw "$name failed. Actual: [$actual]; expected: [$expected]" }
}
foreach ($case in @(
    @{ Input = ''; Expected = '""' },
    @{ Input = 'C:\drawings\part.dwg'; Expected = '"C:\drawings\part.dwg"' },
    @{ Input = 'C:\Two spaces\O''Brien & 100%.dwg'; Expected = '"C:\Two spaces\O''Brien & 100%.dwg"' },
    @{ Input = 'C:\trailing\'; Expected = '"C:\trailing\\"' },
    @{ Input = 'a"b'; Expected = '"a\"b"' },
    @{ Input = 'a\"b'; Expected = '"a\\\"b"' }
)) { Assert-Equal (ConvertTo-WindowsArgument $case.Input) $case.Expected 'CRT quoting' }
Write-Host 'CRT argument cases passed.'
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('EntoCAD Windows smoke space ' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
$originalPath = $env:PATH
$originalSmokeJson = $env:ENTOCAD_SMOKE_JSON
$originalSmokeArgs = $env:ENTOCAD_SMOKE_ARGS
$originalSmokeFail = $env:ENTOCAD_SMOKE_FAIL
$windowsHost = $env:OS -eq 'Windows_NT'
try {
    $RepoRoot = Join-Path $fixture 'checkout'
    $InstallDir = Join-Path $fixture 'installed'
    $bin = Join-Path $fixture 'bin'
    New-Item -ItemType Directory -Path $RepoRoot, $bin | Out-Null
    Set-Content (Join-Path $RepoRoot 'Cargo.toml') "[workspace.package]`nversion = `"1.2.3`""
    $InstalledExe = Join-Path $InstallDir 'EntoCAD.exe'
    $SourcePath = Join-Path $InstallDir 'EntoCAD.source.json'
    $InstalledUninstaller = Join-Path $InstallDir 'uninstall.ps1'
    $ShortcutPath = Join-Path $fixture 'unused.lnk'
    $artifact = Join-Path $fixture 'artifact space.exe'
    [IO.File]::WriteAllText($artifact, 'new-working-program')
    $env:ENTOCAD_SMOKE_JSON = Join-Path $fixture 'cargo.json'
    $env:ENTOCAD_SMOKE_ARGS = Join-Path $fixture 'cargo.args'
    @{ reason = 'compiler-artifact'; target = @{ name = 'mycad' }; executable = $artifact } | ConvertTo-Json -Compress | Set-Content $env:ENTOCAD_SMOKE_JSON
    $fakeCargo = Join-Path $bin 'cargo'
    if ($windowsHost) {
        $fakeCargo += '.cmd'
        [IO.File]::WriteAllText($fakeCargo, @'
@echo off
setlocal DisableDelayedExpansion
if exist "%ENTOCAD_SMOKE_ARGS%" del "%ENTOCAD_SMOKE_ARGS%"
for %%A in (%*) do echo %%~A>> "%ENTOCAD_SMOKE_ARGS%"
if "%ENTOCAD_SMOKE_FAIL%"=="1" exit /b 19
type "%ENTOCAD_SMOKE_JSON%"
'@)
    }
    else {
        [IO.File]::WriteAllText($fakeCargo, @'
#!/bin/sh
printf '%s\n' "$@" > "$ENTOCAD_SMOKE_ARGS"
if [ "$ENTOCAD_SMOKE_FAIL" = 1 ]; then exit 19; fi
cat "$ENTOCAD_SMOKE_JSON"
'@)
        & chmod +x $fakeCargo
        if ($LASTEXITCODE -ne 0) { throw 'Could not make the mock cargo executable' }
    }
    $env:PATH = $bin + [IO.Path]::PathSeparator + $env:PATH
    $env:ENTOCAD_SMOKE_FAIL = ''
    function Assert-AppStopped { }
    function Register-EntoCad { param([string]$Version); if ($script:RegistrationFailure) { throw 'Simulated registration failure' } }
    $script:RegistrationFailure = $true
    $failed = $false
    try { Install-EntoCad } catch { $failed = $true }
    if (-not $failed) { throw 'Expected fresh-install registration failure' }
    if (@(Get-ChildItem -LiteralPath $InstallDir).Count -ne 0) { throw 'Failed fresh installation left program files' }
    $script:RegistrationFailure = $false
    Install-EntoCad
    Assert-Equal ([IO.File]::ReadAllText($InstalledExe)) 'new-working-program' 'fresh install'
    $manifest = Get-Content -Raw $SourcePath | ConvertFrom-Json
    Assert-Equal $manifest.installed_version '1.2.3' 'manifest version'
    Assert-Equal $manifest.repo $RepoRoot 'manifest checkout'
    Assert-Equal $manifest.protocol_version 1 'manifest protocol'
    if (-not (Test-Path -LiteralPath $InstalledUninstaller)) { throw 'Uninstaller missing' }
    Assert-Equal ((Get-Content $env:ENTOCAD_SMOKE_ARGS) -join ',') 'build,--release,-p,mycad,--locked,--message-format=json-render-diagnostics' 'cargo invocation'
    $oldMetadata = [IO.File]::ReadAllText($SourcePath)
    $oldUninstaller = [IO.File]::ReadAllText($InstalledUninstaller)
    [IO.File]::WriteAllText($artifact, 'second-program')
    $script:RegistrationFailure = $true
    $failed = $false
    try { Install-EntoCad } catch { $failed = $true }
    if (-not $failed) { throw 'Expected registration failure' }
    Assert-Equal ([IO.File]::ReadAllText($InstalledExe)) 'new-working-program' 'rollback executable'
    Assert-Equal ([IO.File]::ReadAllText($SourcePath)) $oldMetadata 'rollback metadata'
    Assert-Equal ([IO.File]::ReadAllText($InstalledUninstaller)) $oldUninstaller 'rollback uninstaller'
    if (@(Get-ChildItem -LiteralPath $InstallDir).Count -ne 3) { throw 'Transaction debris remains' }
    $script:RegistrationFailure = $false
    $env:ENTOCAD_SMOKE_FAIL = '1'
    $failed = $false
    try { Install-EntoCad } catch { $failed = $true }
    if (-not $failed) { throw 'Expected build failure' }
    Assert-Equal ([IO.File]::ReadAllText($InstalledExe)) 'new-working-program' 'build failure preserves app'
    $env:ENTOCAD_SMOKE_FAIL = ''
    Install-EntoCad
    Assert-Equal ([IO.File]::ReadAllText($InstalledExe)) 'second-program' 'successful update'
    $WaitPid = 2147483646
    Wait-ForPreviousApp
    $powerShellExecutable = (Get-Process -Id $PID).Path
    $previous = [Diagnostics.Process]::Start($powerShellExecutable, '-NoLogo -NoProfile -Command "Start-Sleep -Milliseconds 100"')
    $WaitPid = $previous.Id
    Wait-ForPreviousApp
    $previous.Dispose()
    Write-Host 'Fresh install, metadata, locked cargo, rollback, build failure, update and PID wait smoke checks passed.'
}
finally {
    $env:PATH = $originalPath
    $env:ENTOCAD_SMOKE_JSON = $originalSmokeJson
    $env:ENTOCAD_SMOKE_ARGS = $originalSmokeArgs
    $env:ENTOCAD_SMOKE_FAIL = $originalSmokeFail
    Remove-Item -LiteralPath $fixture -Recurse -Force
}
