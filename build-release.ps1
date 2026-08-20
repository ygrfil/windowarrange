[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-CargoStep {
    param(
        [Parameter(Mandatory)]
        [string] $Label,

        [Parameter(Mandatory)]
        [string[]] $Arguments
    )

    Write-Host "`n==> $Label" -ForegroundColor Cyan
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE."
    }
}

$repositoryRoot = $PSScriptRoot
$cargoTomlPath = Join-Path $repositoryRoot 'Cargo.toml'
$cargoLockPath = Join-Path $repositoryRoot 'Cargo.lock'
$distPath = Join-Path $repositoryRoot 'dist'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

Push-Location $repositoryRoot
try {
    $cargoToml = [System.IO.File]::ReadAllText($cargoTomlPath)
    $versionPattern = [regex]::new('(?m)^version = "(\d+)\.(\d+)\.(\d+)"\r?$')
    $versionMatch = $versionPattern.Match($cargoToml)
    if (-not $versionMatch.Success) {
        throw 'Could not read the package version from Cargo.toml.'
    }

    $major = [int] $versionMatch.Groups[1].Value
    $minor = [int] $versionMatch.Groups[2].Value
    $patch = [int] $versionMatch.Groups[3].Value
    $version = "$major.$minor.$patch"

    while (Test-Path -LiteralPath (Join-Path $distPath "Table-Arranger-Control-$version.exe")) {
        $patch++
        $version = "$major.$minor.$patch"
    }

    $currentVersion = $versionMatch.Groups[0].Value -replace '^version = "|"\r?$', ''
    if ($version -ne $currentVersion) {
        Write-Host "Advancing release version from $currentVersion to $version." -ForegroundColor Yellow
        $cargoToml = $versionPattern.Replace($cargoToml, "version = `"$version`"", 1)
        [System.IO.File]::WriteAllText($cargoTomlPath, $cargoToml, $utf8NoBom)

        $cargoLock = [System.IO.File]::ReadAllText($cargoLockPath)
        $lockVersionPattern = [regex]::new(
            '(?m)(^name = "clubgg-table-arranger"\r?\nversion = ")[^"]+("$)'
        )
        if (-not $lockVersionPattern.IsMatch($cargoLock)) {
            throw 'Could not find the application package entry in Cargo.lock.'
        }
        $cargoLock = $lockVersionPattern.Replace($cargoLock, "`${1}$version`${2}", 1)
        [System.IO.File]::WriteAllText($cargoLockPath, $cargoLock, $utf8NoBom)
    }

    Invoke-CargoStep 'Checking formatting' @('fmt', '--check')
    Invoke-CargoStep 'Running Clippy' @(
        'clippy', '--bin', 'table-arranger-control', '--all-features', '--', '-D', 'warnings'
    )
    Invoke-CargoStep 'Building locked release (tests are not run)' @(
        'build', '--release', '--locked'
    )

    $sourcePath = (Resolve-Path -LiteralPath 'target\release\table-arranger-control.exe').Path
    [System.IO.Directory]::CreateDirectory($distPath) | Out-Null
    $resolvedDistPath = (Resolve-Path -LiteralPath $distPath).Path
    $currentPath = Join-Path $resolvedDistPath 'Table-Arranger-Control.exe'
    $archivePath = Join-Path $resolvedDistPath "Table-Arranger-Control-$version.exe"

    if (Test-Path -LiteralPath $archivePath) {
        throw "The versioned archive already exists: $archivePath"
    }

    Copy-Item -LiteralPath $sourcePath -Destination $currentPath -Force
    Copy-Item -LiteralPath $sourcePath -Destination $archivePath

    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $sourcePath).Hash
    $currentHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $currentPath).Hash
    $archiveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash
    if ($sourceHash -ne $currentHash -or $sourceHash -ne $archiveHash) {
        throw 'Published executable hashes do not match the verified release build.'
    }

    $productVersion = (Get-Item -LiteralPath $archivePath).VersionInfo.ProductVersion
    if ($productVersion -ne $version) {
        throw "Published product version is $productVersion instead of $version."
    }

    Write-Host "`nRelease $version is ready." -ForegroundColor Green
    Write-Host "Current: $currentPath"
    Write-Host "Archive: $archivePath"
    Write-Host "SHA-256: $sourceHash"
}
finally {
    Pop-Location
}
