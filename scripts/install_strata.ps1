param(
    [Parameter(Mandatory = $true)][string]$Root,
    [Parameter(Mandatory = $true)][string]$Data,
    [ValidateSet('recommended', 'qwen-q2', 'qwen-iq2', 'qwen-iq3xxs', 'qwen-iq3s', 'swift', 'coder')]
    [string]$Model = 'recommended',
    [switch]$CheckOnly
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$env:PYTHONIOENCODING = 'utf-8'

function Send-Event([string]$Kind, [string]$Message) {
    Write-Output ('HUB_EVENT ' + (@{kind = $Kind; message = $Message} | ConvertTo-Json -Compress))
}

function Invoke-Native([string]$Program, [string[]]$Arguments) {
    # Windows PowerShell wraps redirected native stderr in ErrorRecord objects.
    # Keep the output in the log and use the native exit code as the failure signal.
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & $Program @Arguments 2>&1 | ForEach-Object { [string]$_ }
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previous }
    if ($code -ne 0) { throw "Native command failed (exit $code): $Program" }
}

function Get-InstallPlan {
    foreach ($path in @($Root, $Data)) {
        if (-not [IO.Path]::IsPathRooted($path) -or $path -notmatch '^(?:[A-Za-z]:[\\/]|\\\\[^\\]+\\[^\\]+)') {
            throw 'Installation paths must be fully qualified.'
        }
    }
    $destination = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]'\/')
    $models = [IO.Path]::GetFullPath($Data).TrimEnd([char[]]'\/')
    if ($destination -eq [IO.Path]::GetPathRoot($destination).TrimEnd([char[]]'\/')) {
        throw 'Choose a dedicated Strata folder, not a drive root.'
    }
    if ($models -eq $destination -or $models.StartsWith($destination + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Choose a model data folder outside the Strata installation folder.'
    }
    if (Test-Path -LiteralPath $destination) {
        if (-not (Test-Path -LiteralPath $destination -PathType Container) -or
            @(Get-ChildItem -LiteralPath $destination -Force).Count -ne 0) {
            throw 'Choose a new or empty installation folder. Existing installations and partial downloads are not overwritten.'
        }
    }
    if ((Test-Path -LiteralPath $models) -and -not (Test-Path -LiteralPath $models -PathType Container)) {
        throw 'The model data path is not a directory.'
    }
    $setupArguments = @('--yes', '--no-start', '--no-browser', '--host', '127.0.0.1', '--data-dir', $models, '--vision', 'no')
    switch ($Model) {
        'recommended' { $setupArguments += @('--family', 'qwen') }
        'qwen-q2' { $setupArguments += @('--family', 'qwen', '--model', 'Q2_0') }
        'qwen-iq2' { $setupArguments += @('--family', 'qwen', '--model', 'IQ2_XS') }
        'qwen-iq3xxs' { $setupArguments += @('--family', 'qwen', '--model', 'IQ3_XXS') }
        'qwen-iq3s' { $setupArguments += @('--family', 'qwen', '--model', 'IQ3_S') }
        'swift' { $setupArguments += @('--family', 'swift') }
        'coder' { $setupArguments += @('--family', 'coder') }
    }
    return @{root = $destination; data = $models; arguments = $setupArguments}
}

function Find-Python {
    $launcher = Get-Command py.exe -CommandType Application -ErrorAction SilentlyContinue
    $python = Get-Command python.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($launcher) {
        $program = $launcher.Source
        $arguments = @('-3')
    } elseif ($python -and $python.Source -notmatch '[\\/]Microsoft[\\/]WindowsApps[\\/]') {
        $program = $python.Source
        $arguments = @()
    } else {
        $candidates = @(@(313, 312, 311, 310) | ForEach-Object {
            Join-Path $env:LOCALAPPDATA "Programs\Python\Python$_\python.exe"
        } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf })
        if ($candidates.Count -eq 0) { return $null }
        $program = $candidates[0]
        $arguments = @()
    }
    $probe = 'import sys; sys.exit(1) if sys.version_info < (3,10) or sys.maxsize <= 2**32 else print(sys.executable)'
    $result = & $program @arguments -c $probe
    if ($LASTEXITCODE -ne 0) { throw 'The detected Python is unsupported. Install 64-bit Python 3.10 or newer and run setup again.' }
    return ([string]($result | Select-Object -Last 1)).Trim()
}

function Install-Strata($Plan) {
    $git = Get-Command git.exe -CommandType Application -ErrorAction SilentlyContinue
    if (-not $git) { throw 'Git is required. Install Git for Windows from https://git-scm.com/download/win and reopen the hub.' }
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Send-Event 'progress' 'Checking the latest stable Strata release'
    $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/Niko1221/Strata/releases/latest' -TimeoutSec 20 `
        -Headers @{'User-Agent' = 'StrataHub/1.0'; Accept = 'application/vnd.github+json'}
    $tag = [string]$release.tag_name
    if ($release.draft -or $release.prerelease -or $tag -notmatch '^v?\d+\.\d+\.\d+(?:\.\d+)?$' -or
        $release.html_url -ne "https://github.com/Niko1221/Strata/releases/tag/$tag") {
        throw 'GitHub did not return a published stable Strata release.'
    }
    Send-Event 'progress' "Downloading Strata $tag source"
    $parent = Split-Path $Plan.root
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $env:GIT_TERMINAL_PROMPT = '0'
    Invoke-Native $git.Source @('-c', 'core.longpaths=true', 'clone', '--depth', '1', '--branch', $tag,
        '--single-branch', '--no-tags', '--', 'https://github.com/Niko1221/Strata.git', $Plan.root)
    foreach ($name in @('setup.py', 'START-HERE.bat', 'serve\server.py')) {
        if (-not (Test-Path -LiteralPath (Join-Path $Plan.root $name) -PathType Leaf)) { throw "The release is missing $name." }
    }
    Send-Event 'progress' 'Preparing Python'
    $python = Find-Python
    if (-not $python) {
        Send-Event 'progress' 'Installing Python 3.12 for your Windows user account'
        $installer = Join-Path $Plan.root 'python-3.12.10-amd64.exe'
        Invoke-WebRequest -UseBasicParsing -Uri 'https://www.python.org/ftp/python/3.12.10/python-3.12.10-amd64.exe' `
            -OutFile $installer -TimeoutSec 300
        $signature = Get-AuthenticodeSignature -LiteralPath $installer
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)CN=Python Software Foundation(?:,|$)') {
            throw 'The Python installer does not have a valid Python Software Foundation signature.'
        }
        $result = Start-Process -FilePath $installer -ArgumentList @('/quiet', 'InstallAllUsers=0', 'PrependPath=0',
            'Include_launcher=0', 'Include_test=0') -Wait -PassThru -WindowStyle Hidden
        if ($result.ExitCode -ne 0) { throw "Python installation failed (exit $($result.ExitCode))." }
        $python = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312\python.exe'
        if (-not (Test-Path -LiteralPath $python -PathType Leaf)) { throw 'Python installation completed without the expected executable.' }
    }
    $environment = Join-Path $Plan.root '.venv'
    Invoke-Native $python @('-m', 'venv', $environment)
    $runtime = Join-Path $environment 'Scripts\python.exe'
    if (-not (Test-Path -LiteralPath $runtime -PathType Leaf)) { throw 'The Strata Python environment is missing.' }
    Send-Event 'progress' 'Running official setup: engine, dependencies and model download. See the log for details.'
    # Agent connectors require a real local API key. Generate it only for this fresh installation.
    $bytes = New-Object byte[] 32
    $random = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $random.GetBytes($bytes) } finally { $random.Dispose() }
    $key = -join ($bytes | ForEach-Object { $_.ToString('x2') })
    Push-Location $Plan.root
    try {
        Invoke-Native $runtime (@('-u', (Join-Path $Plan.root 'setup.py')) + $Plan.arguments + @('--api-key', $key))
    } finally { Pop-Location }
    if (-not (Test-Path -LiteralPath (Join-Path $Plan.root 'engine\BUILD.json') -PathType Leaf)) {
        throw 'Setup did not produce an engine build manifest.'
    }
    $profiles = @(Get-ChildItem -LiteralPath $Plan.root -Filter 'strata-*.json' -File | Where-Object {
        $config = Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
        $config.args -and $config.model_name
    })
    if ($profiles.Count -eq 0) { throw 'Setup did not produce a usable model profile.' }
    Send-Event 'complete' "Strata $tag installed. Save settings to open the hub. The model is stopped."
}

try {
    $plan = Get-InstallPlan
    if ($CheckOnly) { $plan | ConvertTo-Json -Depth 4; exit 0 }
    Install-Strata $plan
} catch {
    Send-Event 'error' $_.Exception.Message
    exit 1
}
