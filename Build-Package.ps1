#Requires -Version 7.0
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'dist'),
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
function Assert-Publishable([string]$Directory) {
    $privatePaths = @($env:USERPROFILE, $PSScriptRoot, $env:CARGO_HOME, $env:RUSTUP_HOME) |
        Where-Object { $_ } | Select-Object -Unique
    $patterns = @{
        'provider credential' = '(?:sk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}|hf_[A-Za-z0-9]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[A-Z0-9]{16})'
        'private key' = '-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----'
        'personal Windows path' = '(?i)[A-Z]:[\\/]+Users[\\/]+[^\\/\s"<>]+'
    }
    foreach ($file in Get-ChildItem -LiteralPath $Directory -Recurse -File) {
        $bytes = [IO.File]::ReadAllBytes($file.FullName)
        foreach ($encoding in @([Text.Encoding]::UTF8, [Text.Encoding]::Unicode)) {
            $content = $encoding.GetString($bytes)
            foreach ($path in $privatePaths) {
                foreach ($needle in @($path, $path.Replace('\', '/'), $path.Replace('\', '\\'))) {
                    if ($content.Contains($needle, [StringComparison]::OrdinalIgnoreCase)) {
                        throw "Personal build path found in publishable file: $($file.Name)"
                    }
                }
            }
            foreach ($category in $patterns.Keys) {
                if ($content -match $patterns[$category]) {
                    throw "Possible $category found in publishable file: $($file.Name)"
                }
            }
        }
    }
}
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
Push-Location $PSScriptRoot
try {
    if (-not $SkipBuild) {
        $cargoDirectory = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
        $flags = @("--remap-path-prefix=$($env:USERPROFILE)=build-home", "--remap-path-prefix=$cargoDirectory=registry",
            "--remap-path-prefix=$PSScriptRoot=strata-hub", '-C', 'target-feature=+crt-static')
        $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
        cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $executable = Join-Path $PSScriptRoot 'target\release\strata-dashboard.exe'
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw 'Build the release executable first.' }
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $destination = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) $stamp
    $source = Join-Path $destination 'strata-hub-source'
    $release = Join-Path $destination 'strata-hub-windows-x64'
    New-Item -ItemType Directory -Path $source, $release | Out-Null
    # Keep individual paths explicit so new local JSON/configuration files never enter a package implicitly.
    $files = @('Cargo.toml', 'Cargo.lock', 'build.rs', '.gitignore', '.gitattributes', 'README.md',
        'PUBLISHING.md', 'Build-Package.ps1', 'hub-settings.example.json', 'Start-StrataHub.bat', 'Open-Settings.bat',
        'src/config.rs', 'src/data.rs', 'src/hub_ui.rs', 'src/installer.rs', 'src/jobs.rs', 'src/launch.rs',
        'src/main.rs', 'src/models_ui.rs', 'src/profiles.rs', 'src/resources.rs', 'src/settings_ui.rs', 'src/theme.rs', 'src/tools_ui.rs',
        'scripts/agents.py', 'scripts/model_library.py', 'scripts/update_strata.py', 'scripts/install_strata.ps1', 'scripts/Export-LogoIcon.ps1',
        'tests/test_tools.py', 'assets/schema.sql', 'assets/qwen-pricing.json', 'assets/logo-prompt.md',
        'assets/strata-hub-logo.png', 'assets/strata-hub.ico')
    foreach ($name in $files) {
        $target = Join-Path $source $name
        New-Item -ItemType Directory -Path (Split-Path $target) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $target
    }
    foreach ($name in @('README.md', 'PUBLISHING.md', 'Start-StrataHub.bat', 'Open-Settings.bat', 'hub-settings.example.json')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $release
    }
    foreach ($name in @('LICENSE', 'NOTICE')) {
        $file = Join-Path $PSScriptRoot $name
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required license file is missing: $name" }
        foreach ($directory in @($source, $release)) { Copy-Item -LiteralPath $file -Destination $directory }
    }
    Copy-Item -LiteralPath $executable -Destination (Join-Path $release 'StrataDashboard.exe')
    $metadataText = cargo metadata --offline --filter-platform x86_64-pc-windows-msvc --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Could not read dependency licenses.' }
    $metadata = $metadataText | ConvertFrom-Json
    $notices = [Text.StringBuilder]::new("Strata Hub third-party notices`n`nSQLite is bundled through libsqlite3-sys. SQLite itself is in the public domain.`n`n")
    foreach ($package in $metadata.packages | Sort-Object name) {
        if (-not $package.source) { continue }
        [void]$notices.AppendLine(('=' * 72))
        [void]$notices.AppendLine("$($package.name) $($package.version) | $($package.license)")
        [void]$notices.AppendLine($package.repository)
        foreach ($license in Get-ChildItem -LiteralPath (Split-Path $package.manifest_path) -File |
            Where-Object Name -Match '^(LICENSE|LICENCE|COPYING)([-_.]|$)') {
            [void]$notices.AppendLine($license.Name)
            [void]$notices.AppendLine([IO.File]::ReadAllText($license.FullName))
        }
    }
    foreach ($directory in @($source, $release)) {
        [IO.File]::WriteAllText((Join-Path $directory 'THIRD_PARTY_NOTICES.txt'), $notices.ToString(), [Text.UTF8Encoding]::new($false))
        Assert-Publishable $directory
        Compress-Archive -LiteralPath $directory -DestinationPath ($directory + '.zip')
    }
    Get-FileHash -LiteralPath (Join-Path $release 'StrataDashboard.exe') -Algorithm SHA256 |
        Select-Object @{Name='File';Expression={'StrataDashboard.exe'}}, Hash |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'checksums.json') -Encoding utf8
    Write-Output $destination
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
    Pop-Location
}
