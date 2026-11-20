# Run on Windows x64 with PowerShell 7 and Python. Prebuilt releases also need the native Mesh linker.
# Set M034_S03_PREBUILT_RELEASE_DIR to the directory containing both Windows ZIPs and the shared SHA256SUMS.
# Set M034_S03_REQUIRE_SKILLS=1 for prebuilt candidates that include the skill picker.
# Add -InteractiveSkills in a console to exercise the documented irm | iex entry point with real input.
param([switch]$InteractiveSkills)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest

$RootDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $RootDir

$TmpRoot = Join-Path $RootDir '.tmp/m034-s03/windows'
$VerifyRoot = Join-Path $TmpRoot 'verify'
$StageRoot = Join-Path $TmpRoot 'stage'
$ServerRoot = Join-Path $StageRoot 'server'
$HomeRoot = Join-Path $TmpRoot 'home'
$WorkRoot = Join-Path $TmpRoot 'work'
$FixtureDir = Join-Path $RootDir 'scripts/fixtures/m034-s03-installer-smoke'
$InstallScript = Join-Path $RootDir 'website/docs/public/install.ps1'
$RepoInstallScript = Join-Path $RootDir 'tools/install/install.ps1'
$MeshcExe = Join-Path $RootDir 'target/debug/meshc.exe'
$MeshpkgExe = Join-Path $RootDir 'target/debug/meshpkg.exe'
$HostedHelloBuildLog = Join-Path $RootDir '.tmp/m034-s11/t03/diag-download/windows/verify/run/07-hello-build.log'
$PrebuiltReleaseDir = $env:M034_S03_PREBUILT_RELEASE_DIR
$RequireSkills = -not $PrebuiltReleaseDir -or $env:M034_S03_REQUIRE_SKILLS -eq '1' -or $InteractiveSkills
$RunDir = Join-Path $VerifyRoot 'run'
$ServerProcess = $null
$LastStdoutPath = ''
$LastStderrPath = ''
$LastLogPath = ''
$LastExitCode = 0
$Version = ''
$Target = 'x86_64-pc-windows-msvc'
$MeshcArchive = ''
$MeshpkgArchive = ''
$GoodRoot = Join-Path $ServerRoot 'good'
$PriorCargoTargetDir = $null
$HadCargoTargetDir = $false
$PriorMeshRuntimePath = $null

function Stop-LocalServer {
    if ($script:ServerProcess -and -not $script:ServerProcess.HasExited) {
        try {
            Stop-Process -Id $script:ServerProcess.Id -Force -ErrorAction SilentlyContinue
        } catch {
        }
    }
}

# An empty target dir keeps meshc from finding this checkout's runtime: the
# build must link the runtime the installer put in ~/.mesh/lib.
function Get-InstalledBuildTargetDir {
    $dir = Join-Path $WorkRoot 'no-cargo-target'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    return $dir
}

function Push-InstalledBuildEnvironment {
    $existingValue = Get-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    if ($null -ne $existingValue) {
        $script:HadCargoTargetDir = $true
        $script:PriorCargoTargetDir = $existingValue.Value
    } else {
        $script:HadCargoTargetDir = $false
        $script:PriorCargoTargetDir = $null
    }

    $env:CARGO_TARGET_DIR = Get-InstalledBuildTargetDir
    $script:PriorMeshRuntimePath = Get-Item Env:MESH_RT_LIB_PATH -ErrorAction SilentlyContinue
    Remove-Item Env:MESH_RT_LIB_PATH -ErrorAction SilentlyContinue
}

function Pop-InstalledBuildEnvironment {
    if ($script:HadCargoTargetDir) {
        $env:CARGO_TARGET_DIR = $script:PriorCargoTargetDir
    } else {
        Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    }
    if ($null -ne $script:PriorMeshRuntimePath) {
        $env:MESH_RT_LIB_PATH = $script:PriorMeshRuntimePath.Value
    }
}

function Fail-Phase {
    param(
        [string]$Phase,
        [string]$Reason,
        [string]$LogPath = ''
    )

    $message = @("verification drift: $Reason", "first failing phase: $Phase", "artifacts: $RunDir", "staged root: $ServerRoot")
    if ($LogPath -and (Test-Path $LogPath)) {
        $message += "--- $LogPath ---"
        $message += Get-Content $LogPath | Select-Object -First 260
    }
    throw ($message -join "`n")
}

function Get-CommandExitCode {
    $lastExitCodeVar = Get-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
    if ($null -eq $lastExitCodeVar) {
        return 0
    }

    $exitCode = $lastExitCodeVar.Value
    if ($null -eq $exitCode) {
        return 0
    }

    return [int]$exitCode
}

function Set-LastArtifacts {
    param(
        [string]$StdoutPath,
        [string]$StderrPath,
        [string]$LogPath,
        [int]$ExitCode
    )

    $script:LastStdoutPath = $StdoutPath
    $script:LastStderrPath = $StderrPath
    $script:LastLogPath = $LogPath
    $script:LastExitCode = $ExitCode
}

function Combine-CommandLog {
    param(
        [string]$Display,
        [string]$StdoutPath,
        [string]$StderrPath,
        [string]$LogPath,
        [int]$ExitCode
    )

    $content = [System.Collections.Generic.List[string]]::new()
    $content.Add("display: $Display")
    $content.Add("exit_code: $ExitCode")
    $content.Add("stdout_path: $StdoutPath")
    $content.Add("stderr_path: $StderrPath")
    if ((Test-Path $StdoutPath) -and (Get-Item $StdoutPath).Length -gt 0) {
        $content.Add('')
        $content.Add('[stdout]')
        $content.AddRange([string[]](Get-Content $StdoutPath))
    }
    if ((Test-Path $StderrPath) -and (Get-Item $StderrPath).Length -gt 0) {
        $content.Add('')
        $content.Add('[stderr]')
        $content.AddRange([string[]](Get-Content $StderrPath))
    }
    Set-Content -Path $LogPath -Value $content
}

function Invoke-LoggedCommand {
    param(
        [string]$Phase,
        [string]$Label,
        [string]$Display,
        [scriptblock]$Command,
        [switch]$ExpectFailure
    )

    $stdoutPath = Join-Path $script:RunDir "$Label.stdout"
    $stderrPath = Join-Path $script:RunDir "$Label.stderr"
    $logPath = Join-Path $script:RunDir "$Label.log"

    Write-Host "==> [$Phase] $Display"
    & $Command 1> $stdoutPath 2> $stderrPath
    $exitCode = Get-CommandExitCode

    Combine-CommandLog -Display $Display -StdoutPath $stdoutPath -StderrPath $stderrPath -LogPath $logPath -ExitCode $exitCode
    Set-LastArtifacts -StdoutPath $stdoutPath -StderrPath $stderrPath -LogPath $logPath -ExitCode $exitCode

    if ($ExpectFailure) {
        if ($exitCode -eq 0) {
            Fail-Phase $Phase "$Display unexpectedly succeeded" $logPath
        }
        return
    }

    if ($exitCode -ne 0) {
        Fail-Phase $Phase "$Display failed" $logPath
    }
}

function Assert-LogContains {
    param(
        [string]$Phase,
        [string]$Needle,
        [string]$LogPath
    )

    if (-not (Select-String -Path $LogPath -SimpleMatch $Needle -Quiet)) {
        Fail-Phase $Phase "expected to find '$Needle' in $LogPath" $LogPath
    }
}

function Get-RepoVersion {
    $meshc = (Get-Content (Join-Path $RootDir 'compiler/meshc/Cargo.toml') -Raw) -match 'version = "([^"]+)"' | Out-Null
    $meshcVersion = $Matches[1]
    $meshpkg = (Get-Content (Join-Path $RootDir 'compiler/meshpkg/Cargo.toml') -Raw) -match 'version = "([^"]+)"' | Out-Null
    $meshpkgVersion = $Matches[1]
    if ($meshcVersion -ne $meshpkgVersion) {
        throw "meshc ($meshcVersion) and meshpkg ($meshpkgVersion) versions diverged"
    }
    return $meshcVersion
}

function Get-Sha256 {
    param([string]$Path)
    return (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = ($listener.LocalEndpoint).Port
    $listener.Stop()
    return $port
}

function Get-PythonCommand {
    foreach ($name in @('python', 'py')) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) { return $cmd.Name }
    }
    throw 'python or py is required to host staged release assets'
}

function Find-SingleFile {
    param(
        [string]$Dir,
        [string]$Filter
    )

    $matches = @(Get-ChildItem -Path $Dir -Filter $Filter -File -ErrorAction SilentlyContinue)
    if ($matches.Count -ne 1) {
        throw "expected exactly one match for $Dir/$Filter, found $($matches.Count)"
    }

    return $matches[0].FullName
}

function Get-VersionFromArchiveName {
    param(
        [string]$Prefix,
        [string]$ArchiveName,
        [string]$Target,
        [string]$Extension
    )

    $expectedPrefix = "$Prefix-v"
    $expectedSuffix = "-$Target.$Extension"
    if (-not $ArchiveName.StartsWith($expectedPrefix) -or -not $ArchiveName.EndsWith($expectedSuffix)) {
        throw "could not infer version from $ArchiveName"
    }

    return $ArchiveName.Substring($expectedPrefix.Length, $ArchiveName.Length - $expectedPrefix.Length - $expectedSuffix.Length)
}

function New-ZipArchive {
    param(
        [string]$ArchivePath,
        [string]$SourcePath,
        [string]$EntryName,
        [string[]]$ExtraPaths = @()
    )

    $tmpDir = Join-Path $StageRoot ("zip-" + [System.Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
    Copy-Item $SourcePath (Join-Path $tmpDir $EntryName)
    $entries = @(Join-Path $tmpDir $EntryName)
    foreach ($extra in $ExtraPaths) {
        $entry = Join-Path $tmpDir (Split-Path $extra -Leaf)
        Copy-Item $extra $entry
        $entries += $entry
    }
    Compress-Archive -Path $entries -DestinationPath $ArchivePath -Force
    Remove-Item -Recurse -Force $tmpDir
}

function Write-ReleaseJson {
    param(
        [string]$Path,
        [string]$MeshcArchive,
        [string]$MeshpkgArchive,
        [string]$Version
    )

    @{
        tag_name = "v$Version"
        name = 'M034 S03 staged release'
        assets = @(
            @{ name = $MeshcArchive },
            @{ name = $MeshpkgArchive },
            @{ name = 'SHA256SUMS' }
        )
    } | ConvertTo-Json -Depth 5 | Set-Content -Path $Path
}

function Setup-PrebuiltReleaseAssets {
    param([string]$AssetDir)

    if (-not (Test-Path $AssetDir -PathType Container)) {
        Fail-Phase 'setup' "prebuilt release asset dir was missing: $AssetDir"
    }

    $meshcSource = Find-SingleFile -Dir $AssetDir -Filter "meshc-v*-$Target.zip"
    $meshpkgSource = Find-SingleFile -Dir $AssetDir -Filter "meshpkg-v*-$Target.zip"
    $checksumSource = Join-Path $AssetDir 'SHA256SUMS'
    if (-not (Test-Path $checksumSource -PathType Leaf)) {
        Fail-Phase 'setup' "missing SHA256SUMS in $AssetDir"
    }

    $script:MeshcArchive = Split-Path $meshcSource -Leaf
    $script:MeshpkgArchive = Split-Path $meshpkgSource -Leaf
    $meshcVersion = Get-VersionFromArchiveName -Prefix 'meshc' -ArchiveName $script:MeshcArchive -Target $Target -Extension 'zip'
    $meshpkgVersion = Get-VersionFromArchiveName -Prefix 'meshpkg' -ArchiveName $script:MeshpkgArchive -Target $Target -Extension 'zip'
    if ($meshcVersion -ne $meshpkgVersion) {
        Fail-Phase 'setup' "meshc ($meshcVersion) and meshpkg ($meshpkgVersion) archive versions diverged"
    }
    $script:Version = $meshcVersion

    New-Item -ItemType Directory -Path (Join-Path $GoodRoot 'api/releases'), (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)") -Force | Out-Null
    Copy-Item $meshcSource (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/$($script:MeshcArchive)")
    Copy-Item $meshpkgSource (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/$($script:MeshpkgArchive)")
    Copy-Item $checksumSource (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/SHA256SUMS")
    Write-ReleaseJson -Path (Join-Path $GoodRoot 'api/releases/latest.json') -MeshcArchive $script:MeshcArchive -MeshpkgArchive $script:MeshpkgArchive -Version $script:Version
}

function Setup-LocalReleaseAssets {
    $script:Version = Get-RepoVersion
    $script:MeshcArchive = "meshc-v$($script:Version)-$Target.zip"
    $script:MeshpkgArchive = "meshpkg-v$($script:Version)-$Target.zip"

    Invoke-LoggedCommand -Phase 'tooling' -Label '03-build-tooling' -Display 'cargo build --locked -q -p mesh-rt -p mesh-test-rt -p meshc -p meshpkg' -Command {
        cargo build --locked -q --target-dir (Join-Path $RootDir 'target') -p mesh-rt -p mesh-test-rt -p meshc -p meshpkg
    }

    $runtimeLibs = @('mesh_rt.lib', 'mesh_test_rt.lib') | ForEach-Object { Join-Path $RootDir "target/debug/$_" }
    foreach ($lib in $runtimeLibs) {
        if (-not (Test-Path $lib)) { Fail-Phase 'tooling' "$lib was not built" $LastLogPath }
    }
    if (-not (Test-Path $MeshcExe)) { Fail-Phase 'tooling' 'meshc.exe was not built' $LastLogPath }
    if (-not (Test-Path $MeshpkgExe)) { Fail-Phase 'tooling' 'meshpkg.exe was not built' $LastLogPath }

    New-Item -ItemType Directory -Path (Join-Path $GoodRoot 'api/releases'), (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)") -Force | Out-Null

    New-ZipArchive -ArchivePath (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/$($script:MeshcArchive)") -SourcePath $MeshcExe -EntryName 'meshc.exe' -ExtraPaths $runtimeLibs
    New-ZipArchive -ArchivePath (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/$($script:MeshpkgArchive)") -SourcePath $MeshpkgExe -EntryName 'meshpkg.exe'
    $meshcSha = Get-Sha256 (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/$($script:MeshcArchive)")
    $meshpkgSha = Get-Sha256 (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/$($script:MeshpkgArchive)")
    Set-Content -Path (Join-Path $GoodRoot "hyperpush-org/mesh-lang/releases/download/v$($script:Version)/SHA256SUMS") -Value @(
        "$meshcSha  ./$($script:MeshcArchive)",
        "$meshpkgSha  ./$($script:MeshpkgArchive)"
    )
    Write-ReleaseJson -Path (Join-Path $GoodRoot 'api/releases/latest.json') -MeshcArchive $script:MeshcArchive -MeshpkgArchive $script:MeshpkgArchive -Version $script:Version
}

function Start-LocalServer {
    param([int]$Port)

    $python = Get-PythonCommand
    $script:ServerProcess = Start-Process -FilePath $python -ArgumentList @('-m', 'http.server', $Port, '--bind', '127.0.0.1', '--directory', ('"{0}"' -f $ServerRoot)) -RedirectStandardOutput (Join-Path $RunDir 'http-server.stdout') -RedirectStandardError (Join-Path $RunDir 'http-server.stderr') -PassThru

    $url = "http://127.0.0.1:$Port/"
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        try {
            Invoke-WebRequest -Uri $url -TimeoutSec 2 | Out-Null
            return
        } catch {
            Start-Sleep -Milliseconds 250
        }
    }

    Fail-Phase 'server' 'local staged release server did not become ready' (Join-Path $RunDir 'http-server.stderr')
}

function Test-TamperedReleaseArchives {
    param([string]$ServerUrl)

    $releaseBase = 'hyperpush-org/mesh-lang/releases/download'
    $goodAssets = Join-Path $GoodRoot "$releaseBase/v$Version"
    $tamperedAssets = Join-Path $ServerRoot "tampered/$releaseBase/v$Version"
    New-Item -ItemType Directory -Path $tamperedAssets -Force | Out-Null
    foreach ($file in @('SHA256SUMS', $MeshcArchive, $MeshpkgArchive)) {
        Copy-Item -LiteralPath (Join-Path $goodAssets $file) -Destination $tamperedAssets
    }

    $goodHome = $env:USERPROFILE
    $goodBaseUrl = $env:MESH_INSTALL_RELEASE_BASE_URL
    try {
        $env:MESH_INSTALL_RELEASE_BASE_URL = "$ServerUrl/tampered/$releaseBase"
        foreach ($archive in @($MeshcArchive, $MeshpkgArchive)) {
            $tool = ($archive -split '-v', 2)[0]
            $archivePath = Join-Path $tamperedAssets $archive
            $stream = [IO.File]::Open($archivePath, [IO.FileMode]::Open)
            try {
                $firstByte = $stream.ReadByte()
                $stream.Position = 0
                $stream.WriteByte([byte]($firstByte -bxor 1))
            } finally {
                $stream.Dispose()
            }

            $env:USERPROFILE = Join-Path $HomeRoot "tampered-$tool"
            New-Item -ItemType Directory -Path $env:USERPROFILE -Force | Out-Null
            Invoke-LoggedCommand -Phase 'checksum' -Label "09-tampered-$tool" -Display "Install with tampered $archive" -ExpectFailure -Command {
                pwsh -NoProfile -File $script:InstallScript -Version $script:Version -Yes
            }
            Assert-LogContains -Phase 'checksum' -Needle 'Checksum verification failed' -LogPath $LastLogPath
            Assert-LogContains -Phase 'checksum' -Needle $archive -LogPath $LastLogPath
            if (Test-Path (Join-Path $env:USERPROFILE '.mesh/version')) {
                Fail-Phase 'checksum' "tampered $archive was recorded as installed" $LastLogPath
            }
            Copy-Item -LiteralPath (Join-Path $goodAssets $archive) -Destination $archivePath -Force
        }
    } finally {
        $env:USERPROFILE = $goodHome
        $env:MESH_INSTALL_RELEASE_BASE_URL = $goodBaseUrl
    }
}

function Assert-NoSkillWrites {
    param([string]$Root, [string]$LogPath)

    foreach ($hostDir in @('.agents', '.claude', '.cursor', '.gemini', '.copilot', '.github')) {
        if (Test-Path -LiteralPath (Join-Path $Root $hostDir)) {
            Fail-Phase 'skills' "unexpected skill write under $Root\$hostDir" $LogPath
        }
    }
}

function New-OldSkillFixture {
    param([string]$Path, [string]$Scope)

    $name = [Text.Encoding]::UTF8.GetBytes('SKILL.md')
    $content = [Text.Encoding]::UTF8.GetBytes("# Older Mesh guide`n")
    [byte[]]$digestInput = [BitConverter]::GetBytes([long]$name.Length) + $name +
        [BitConverter]::GetBytes([long]$content.Length) + $content
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = [BitConverter]::ToString($sha256.ComputeHash($digestInput)).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha256.Dispose()
    }
    New-Item -ItemType Directory -Path $Path -Force | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $Path 'SKILL.md'), $content)
    @{ schema = 1; scope = $Scope; digest = $digest } | ConvertTo-Json -Compress |
        Set-Content -LiteralPath (Join-Path $Path '.mesh-install.json')
}

function Get-SkillFixtureFingerprint {
    param([string]$Path)

    return (Get-ChildItem -LiteralPath $Path -File -Force -Recurse | Sort-Object FullName | ForEach-Object {
        "$($_.FullName) $(Get-Sha256 $_.FullName)"
    }) -join "`n"
}

function Test-SkillUpgrade {
    param([string]$InstallHome, [string]$Project)

    $clean = Join-Path $InstallHome '.agents/skills/mesh'
    $edited = Join-Path $InstallHome '.claude/skills/mesh'
    $pinned = Join-Path $Project '.agents/skills/mesh'
    New-OldSkillFixture -Path $clean -Scope 'account'
    New-OldSkillFixture -Path $edited -Scope 'account'
    New-OldSkillFixture -Path $pinned -Scope 'project'
    Add-Content -LiteralPath (Join-Path $edited 'SKILL.md') -Value 'Local instructions.'
    $editedBefore = Get-SkillFixtureFingerprint $edited
    $pinnedBefore = Get-SkillFixtureFingerprint $pinned
    $oldDigest = (Get-Content -LiteralPath (Join-Path $clean '.mesh-install.json') -Raw | ConvertFrom-Json).digest

    # Exercise the version-change path using seeded prior state and real candidate archives.
    $versionFile = Join-Path $InstallHome '.mesh/version'
    Set-Content -LiteralPath $versionFile -Value '0.0.0-fixture' -NoNewline
    Push-Location -LiteralPath $Project
    try {
        Invoke-LoggedCommand -Phase 'skills' -Label '11-skill-upgrade' -Display 'Upgrade seeded skill fixtures with the PowerShell installer -Yes' -Command {
            pwsh -NoProfile -File $script:InstallScript -Yes
        }
    } finally {
        Pop-Location
    }

    Assert-LogContains -Phase 'skills' -Needle "Updating meshc and meshpkg from v0.0.0-fixture to v$Version" -LogPath $LastLogPath
    Assert-LogContains -Phase 'skills' -Needle "Kept $edited" -LogPath $LastLogPath
    if ((Get-Content -LiteralPath $versionFile -Raw).Trim() -cne $Version) {
        Fail-Phase 'skills' 'the upgrade did not write the candidate version' $LastLogPath
    }
    $receipt = Get-Content -LiteralPath (Join-Path $clean '.mesh-install.json') -Raw | ConvertFrom-Json
    $guide = & (Join-Path $InstallHome '.mesh/bin/meshc.exe') guide syntax
    $guideHeader = "Mesh $Version guide (SHA-256 $($receipt.digest))"
    if ($LASTEXITCODE -ne 0 -or $receipt.scope -cne 'account' -or $receipt.digest -cnotmatch '^[0-9a-f]{64}$' -or
        $receipt.digest -ceq $oldDigest -or $guide -cnotcontains $guideHeader -or
        -not (Test-Path -LiteralPath (Join-Path $clean 'references/syntax.md') -PathType Leaf)) {
        Fail-Phase 'skills' 'the clean account copy did not refresh to the installed compiler guide' $LastLogPath
    }
    if ((Get-SkillFixtureFingerprint $edited) -cne $editedBefore -or (Get-SkillFixtureFingerprint $pinned) -cne $pinnedBefore) {
        Fail-Phase 'skills' 'the upgrade changed an edited account skill or a pinned project skill' $LastLogPath
    }
}

function Test-InteractiveSkills {
    param([string]$Project, [string]$ServerUrl)

    # Give the staged copy a text MIME type so Invoke-RestMethod returns script text.
    $env:M034_S03_INSTALLER_URL = "$ServerUrl/good/install.txt"
    Copy-Item -LiteralPath $InstallScript -Destination (Join-Path $GoodRoot 'install.txt')
    Push-Location -LiteralPath $Project
    try {
        foreach ($choice in @('decline', 'accept')) {
            $env:USERPROFILE = Join-Path $HomeRoot $choice
            New-Item -ItemType Directory -Path $env:USERPROFILE -Force | Out-Null
            $transcript = Join-Path $RunDir "12-interactive-$choice.log"
            $env:M034_S03_TRANSCRIPT = $transcript
            if ($choice -eq 'decline') {
                Write-Host 'At the installer prompt, enter n.'
            } else {
                Write-Host 'Enter y at the installer prompt, 1 for Codex, 1 for Account, and y to confirm.'
            }
            powershell.exe -NoProfile -ExecutionPolicy Bypass -Command '
                $ErrorActionPreference = [System.Management.Automation.ActionPreference]::Stop
                $ProgressPreference = [System.Management.Automation.ActionPreference]::SilentlyContinue
                Start-Transcript -LiteralPath $env:M034_S03_TRANSCRIPT | Out-Null
                try { irm $env:M034_S03_INSTALLER_URL | iex }
                finally { Stop-Transcript | Out-Null }
            '
            if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.mesh/version'))) {
                Fail-Phase 'skills' "interactive $choice installation failed" $transcript
            }
            # Windows PowerShell does not record Read-Host prompts in the transcript.
            # The console driver checks each prompt before sending its response.
            Assert-NoSkillWrites -Root $Project -LogPath $transcript
            if ($choice -eq 'decline') {
                Assert-LogContains -Phase 'skills' -Needle "Optional: run 'meshc skills setup'" -LogPath $transcript
                Assert-NoSkillWrites -Root $env:USERPROFILE -LogPath $transcript
            } else {
                $skillPath = Join-Path $env:USERPROFILE '.agents/skills/mesh'
                $receiptPath = Join-Path $skillPath '.mesh-install.json'
                if (-not (Test-Path -LiteralPath (Join-Path $skillPath 'SKILL.md') -PathType Leaf) -or
                    -not (Test-Path -LiteralPath $receiptPath -PathType Leaf) -or
                    (Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json).scope -cne 'account') {
                    Fail-Phase 'skills' 'accepting did not install the Codex account skill' $transcript
                }
            }
        }
    } finally {
        Remove-Item Env:M034_S03_INSTALLER_URL, Env:M034_S03_TRANSCRIPT -ErrorAction SilentlyContinue
        Pop-Location
    }
}

function Test-ObjectProperty {
    param(
        [object]$Object,
        [string]$Name
    )

    if ($null -eq $Object) {
        return $false
    }

    return $Object.PSObject.Properties.Name -contains $Name
}

function Get-LoggedCommandMetadata {
    param([string]$LogPath)

    $meta = [ordered]@{
        exists = $false
        valid = $false
        logPath = $LogPath
        display = $null
        exitCode = $null
        stdoutPath = $null
        stderrPath = $null
        missingFields = @()
    }

    if (-not (Test-Path $LogPath -PathType Leaf)) {
        return [pscustomobject]$meta
    }

    $meta.exists = $true
    foreach ($line in (Get-Content $LogPath)) {
        if ($line -like 'display: *') {
            $meta.display = $line.Substring(9)
        } elseif ($line -like 'exit_code: *') {
            $raw = $line.Substring(11)
            if ($raw -match '^-?\d+$') {
                $meta.exitCode = [int]$raw
            }
        } elseif ($line -like 'stdout_path: *') {
            $meta.stdoutPath = $line.Substring(13)
        } elseif ($line -like 'stderr_path: *') {
            $meta.stderrPath = $line.Substring(13)
        }
    }

    foreach ($field in @('display', 'exitCode', 'stdoutPath', 'stderrPath')) {
        if ($null -eq $meta[$field] -or $meta[$field] -eq '') {
            $meta.missingFields += $field
        }
    }

    $meta.valid = ($meta.missingFields.Count -eq 0)
    return [pscustomobject]$meta
}

function Get-BuildTraceInfo {
    param([string]$TracePath)

    $info = [ordered]@{
        exists = $false
        valid = $false
        path = $TracePath
        parseError = $null
        data = $null
    }

    if (-not (Test-Path $TracePath -PathType Leaf)) {
        return [pscustomobject]$info
    }

    $info.exists = $true
    try {
        $info.data = Get-Content $TracePath -Raw | ConvertFrom-Json
        $info.valid = $true
    } catch {
        $info.parseError = $_.Exception.Message
    }

    return [pscustomobject]$info
}

function Get-InstalledBuildClassification {
    param(
        [int]$ExitCode,
        [object]$TraceInfo
    )

    if ($ExitCode -eq 0) {
        return 'success'
    }

    if (-not $TraceInfo.exists -or -not $TraceInfo.valid) {
        return 'pre-object'
    }

    $trace = $TraceInfo.data
    $objectEmitted = (Test-ObjectProperty -Object $trace -Name 'objectEmissionCompleted') -and [bool]$trace.objectEmissionCompleted
    if (-not $objectEmitted) {
        return 'pre-object'
    }

    $runtimeMissing = (Test-ObjectProperty -Object $trace -Name 'runtimeLibraryExists') -and ($trace.runtimeLibraryExists -eq $false)
    if ($runtimeMissing) {
        return 'runtime-lookup'
    }

    $linkAttempted = ((Test-ObjectProperty -Object $trace -Name 'linkStarted') -and [bool]$trace.linkStarted) -or ((Test-ObjectProperty -Object $trace -Name 'linkerProgram') -and $null -ne $trace.linkerProgram -and $trace.linkerProgram -ne '')
    if ($linkAttempted) {
        return 'link-time'
    }

    return 'pre-object'
}

function Get-DiagnosticEvidenceNote {
    param(
        [string]$Classification,
        [object]$HostedInfo,
        [object]$TraceInfo
    )

    if (-not $HostedInfo.exists) {
        return 'Hosted S11 hello-build anchor is missing.'
    }

    if (-not $HostedInfo.valid) {
        return "Hosted S11 hello-build anchor is malformed: $($HostedInfo.missingFields -join ', ')."
    }

    if (-not $TraceInfo.exists) {
        return 'No local build trace was recorded, so the current classification stays at the earliest observable phase.'
    }

    if (-not $TraceInfo.valid) {
        return "Local build trace was malformed: $($TraceInfo.parseError)."
    }

    if ($Classification -eq 'link-time') {
        return 'Object emission completed and the linker boundary was reached.'
    }

    if ($Classification -eq 'runtime-lookup') {
        return 'Object emission completed, but runtime discovery stayed unresolved before linker invocation.'
    }

    if ($Classification -eq 'success') {
        return 'Installed build completed successfully with full trace coverage.'
    }

    return 'The current evidence does not show completed object emission.'
}

function Write-InstalledBuildContextLog {
    param(
        [string]$Path,
        [string]$InstalledMeshcPath,
        [string]$InstalledMeshpkgPath,
        [string]$TracePath,
        [string]$HelloExePath
    )

    $llvmPrefix = $env:LLVM_SYS_211_PREFIX
    if (-not $llvmPrefix) {
        $llvmPrefix = 'unset'
    }
    $cargoTargetDir = $env:CARGO_TARGET_DIR
    if (-not $cargoTargetDir) {
        $cargoTargetDir = 'unset'
    }
    $meshRtLibPath = $env:MESH_RT_LIB_PATH
    if (-not $meshRtLibPath) {
        $meshRtLibPath = 'unset'
    }

    Set-Content -Path $Path -Value @(
        "installed_meshc=$InstalledMeshcPath",
        "installed_meshpkg=$InstalledMeshpkgPath",
        "trace_path=$TracePath",
        "output_path=$HelloExePath",
        "llvm_sys_211_prefix=$llvmPrefix",
        "cargo_target_dir=$cargoTargetDir",
        "mesh_rt_lib_path=$meshRtLibPath"
    )
}

function Write-InstalledBuildDiagnosticSummary {
    param(
        [string]$SummaryPath,
        [string]$BuildLogPath,
        [string]$StdoutPath,
        [string]$StderrPath,
        [string]$TracePath,
        [string]$BuildContextLogPath,
        [string]$InstalledMeshcPath,
        [string]$InstalledMeshpkgPath,
        [string]$HostedLogPath
    )

    $buildInfo = Get-LoggedCommandMetadata -LogPath $BuildLogPath
    $hostedInfo = Get-LoggedCommandMetadata -LogPath $HostedLogPath
    $traceInfo = Get-BuildTraceInfo -TracePath $TracePath
    $classification = Get-InstalledBuildClassification -ExitCode ($buildInfo.exitCode ?? -1) -TraceInfo $traceInfo
    $evidenceNote = Get-DiagnosticEvidenceNote -Classification $classification -HostedInfo $hostedInfo -TraceInfo $traceInfo

    $summaryDir = Split-Path -Parent $SummaryPath
    if ($summaryDir) {
        New-Item -ItemType Directory -Path $summaryDir -Force | Out-Null
    }

    $traceData = $null
    if ($traceInfo.valid) {
        $traceData = $traceInfo.data
    }

    $payload = [ordered]@{
        generatedAt = [DateTimeOffset]::UtcNow.ToString('o')
        build = [ordered]@{
            classification = $classification
            exitCode = $buildInfo.exitCode
            buildLogPath = $BuildLogPath
            stdoutPath = $StdoutPath
            stderrPath = $StderrPath
            tracePath = $TracePath
            traceExists = $traceInfo.exists
            traceValid = $traceInfo.valid
            traceParseError = $traceInfo.parseError
            buildContextLogPath = $BuildContextLogPath
            installedMeshcPath = $InstalledMeshcPath
            installedMeshpkgPath = $InstalledMeshpkgPath
            llvmSys211Prefix = $env:LLVM_SYS_211_PREFIX
            cargoTargetDir = $env:CARGO_TARGET_DIR
            lastStage = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'lastStage')) { $traceData.lastStage } else { $null }
            objectEmissionStarted = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'objectEmissionStarted')) { $traceData.objectEmissionStarted } else { $null }
            objectEmissionCompleted = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'objectEmissionCompleted')) { $traceData.objectEmissionCompleted } else { $null }
            objectExistsAfterEmit = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'objectExistsAfterEmit')) { $traceData.objectExistsAfterEmit } else { $null }
            runtimeLibraryPath = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'runtimeLibraryPath')) { $traceData.runtimeLibraryPath } else { $null }
            meshRtLibPath = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'meshRtLibPath')) { $traceData.meshRtLibPath } else { $null }
            runtimeLibraryExists = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'runtimeLibraryExists')) { $traceData.runtimeLibraryExists } else { $null }
            linkerProgram = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'linkerProgram')) { $traceData.linkerProgram } else { $null }
            linkStarted = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'linkStarted')) { $traceData.linkStarted } else { $null }
            linkCompleted = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'linkCompleted')) { $traceData.linkCompleted } else { $null }
            traceError = if ($traceData -and (Test-ObjectProperty -Object $traceData -Name 'error')) { $traceData.error } else { $null }
        }
        hostedAnchor = [ordered]@{
            logPath = $HostedLogPath
            exists = $hostedInfo.exists
            valid = $hostedInfo.valid
            exitCode = $hostedInfo.exitCode
            missingFields = $hostedInfo.missingFields
        }
        evidenceNote = $evidenceNote
    }

    $payload | ConvertTo-Json -Depth 10 | Set-Content -Path $SummaryPath
    return [pscustomobject]$payload
}

function Invoke-InstalledBuildCommand {
    param(
        [string]$Phase,
        [string]$Label,
        [string]$Display,
        [scriptblock]$Command,
        [string]$TracePath,
        [string]$SummaryPath,
        [string]$BuildContextLogPath,
        [string]$InstalledMeshcPath,
        [string]$InstalledMeshpkgPath,
        [string]$HostedLogPath
    )

    $stdoutPath = Join-Path $script:RunDir "$Label.stdout"
    $stderrPath = Join-Path $script:RunDir "$Label.stderr"
    $logPath = Join-Path $script:RunDir "$Label.log"

    New-Item -ItemType Directory -Path (Split-Path -Parent $TracePath) -Force | Out-Null
    Remove-Item $TracePath -Force -ErrorAction SilentlyContinue
    $env:MESH_BUILD_TRACE_PATH = $TracePath

    Write-Host "==> [$Phase] $Display"
    try {
        & $Command 1> $stdoutPath 2> $stderrPath
        $exitCode = Get-CommandExitCode
    } finally {
        Remove-Item Env:MESH_BUILD_TRACE_PATH -ErrorAction SilentlyContinue
    }

    Combine-CommandLog -Display $Display -StdoutPath $stdoutPath -StderrPath $stderrPath -LogPath $logPath -ExitCode $exitCode
    Set-LastArtifacts -StdoutPath $stdoutPath -StderrPath $stderrPath -LogPath $logPath -ExitCode $exitCode
    Write-InstalledBuildDiagnosticSummary -SummaryPath $SummaryPath -BuildLogPath $logPath -StdoutPath $stdoutPath -StderrPath $stderrPath -TracePath $TracePath -BuildContextLogPath $BuildContextLogPath -InstalledMeshcPath $InstalledMeshcPath -InstalledMeshpkgPath $InstalledMeshpkgPath -HostedLogPath $HostedLogPath | Out-Null

    if ($exitCode -ne 0) {
        Fail-Phase $Phase "$Display failed" $logPath
    }
}

if ($env:M034_S03_LIB_ONLY -eq '1') {
    return
}

if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'This release proof requires native Windows x64 and PowerShell 7.'
}
if ($InteractiveSkills -and [Console]::IsInputRedirected) {
    throw 'Run -InteractiveSkills from a Windows console with keyboard input.'
}

$originalProfile = $env:USERPROFILE
$userEnvironment = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$pathOptions = [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
$originalUserPath = if ($userEnvironment) { $userEnvironment.GetValue('Path', $null, $pathOptions) }
$originalReadPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$originalPathKind = if ($null -ne $originalUserPath) {
    $userEnvironment.GetValueKind('Path')
} else {
    [Microsoft.Win32.RegistryValueKind]::String
}

try {
    Remove-Item -Recurse -Force $TmpRoot -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $RunDir, $StageRoot, $HomeRoot, $WorkRoot -Force | Out-Null

    $script:RunDir = $RunDir
    $script:ServerRoot = $ServerRoot

    Invoke-LoggedCommand -Phase 'contract' -Label '01-ps1-diff' -Display 'Compare canonical and repo-local PowerShell installers' -Command {
        $canonicalHash = (Get-FileHash -Path $InstallScript -Algorithm SHA256).Hash
        $repoHash = (Get-FileHash -Path $RepoInstallScript -Algorithm SHA256).Hash
        if ($canonicalHash -ne $repoHash) {
            throw 'PowerShell installer copies drifted'
        }
        Write-Output "sha256=$canonicalHash"
    }
    if (-not (Test-Path $InstallScript)) { Fail-Phase 'contract' 'install.ps1 missing' }

    Invoke-LoggedCommand -Phase 'contract' -Label '02-ps1-contract' -Display 'Verify PowerShell installer covers meshpkg and staged hooks' -Command {
        Select-String -Path $InstallScript -Pattern 'hyperpush-org/mesh-lang', 'meshpkg', 'MESH_INSTALL_RELEASE_API_URL', 'MESH_INSTALL_RELEASE_BASE_URL', 'MESH_INSTALL_STRICT_PROOF' | ForEach-Object { $_.Line }
    }
    foreach ($needle in @('hyperpush-org/mesh-lang', 'meshpkg', 'MESH_INSTALL_RELEASE_API_URL', 'MESH_INSTALL_RELEASE_BASE_URL', 'MESH_INSTALL_STRICT_PROOF')) {
        Assert-LogContains -Phase 'contract' -Needle $needle -LogPath $LastLogPath
    }

    if ($PrebuiltReleaseDir) {
        Setup-PrebuiltReleaseAssets -AssetDir $PrebuiltReleaseDir
    } else {
        Setup-LocalReleaseAssets
    }

    Set-Content -Path (Join-Path $RunDir '00-context.log') -Value @(
        "version=$Version",
        "target=$Target",
        "prebuilt_release_dir=$($PrebuiltReleaseDir ?? 'none')",
        "verify_root=$RunDir",
        "stage_root=$StageRoot",
        "fixture_dir=$FixtureDir"
    )

    Get-ChildItem -Path $ServerRoot -File -Recurse | Sort-Object FullName | ForEach-Object { $_.FullName.Replace("$RootDir\", '') } | Set-Content -Path (Join-Path $RunDir 'staged-layout.txt')

    $serverPort = Get-FreePort
    Start-LocalServer -Port $serverPort
    $serverUrl = "http://127.0.0.1:$serverPort"
    $goodApiUrl = "$serverUrl/good/api/releases/latest.json"
    $goodBaseUrl = "$serverUrl/good/hyperpush-org/mesh-lang/releases/download"

    Set-Content -Path (Join-Path $RunDir 'server-urls.log') -Value @(
        "server_url=$serverUrl",
        "good_api_url=$goodApiUrl",
        "good_base_url=$goodBaseUrl"
    )

    $env:MESH_INSTALL_RELEASE_API_URL = $goodApiUrl
    $env:MESH_INSTALL_RELEASE_BASE_URL = $goodBaseUrl
    $env:MESH_INSTALL_STRICT_PROOF = '1'
    $env:MESH_INSTALL_DOWNLOAD_TIMEOUT_SEC = '20'

    $goodHome = Join-Path $HomeRoot 'good'
    $noConsoleHome = Join-Path $HomeRoot 'no-console'
    $skillsProject = Join-Path $WorkRoot 'skills-project'
    New-Item -ItemType Directory -Path $goodHome, $noConsoleHome, $skillsProject -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $FixtureDir 'mesh.toml') -Destination $skillsProject

    Push-Location -LiteralPath $skillsProject
    try {
        $env:USERPROFILE = $noConsoleHome
        Invoke-LoggedCommand -Phase 'install' -Label '10-install-redirected' -Display 'Install with redirected stdin and no -Yes' -Command {
            'y' | pwsh -NoProfile -File $script:InstallScript
        }
        if (-not (Test-Path (Join-Path $noConsoleHome '.mesh/version')) -or
            (Get-Content (Join-Path $noConsoleHome '.mesh/version') -Raw).Trim() -cne $Version) {
            Fail-Phase 'install' 'installation with redirected stdin did not write the candidate version' $LastLogPath
        }
        Assert-NoSkillWrites -Root $noConsoleHome -LogPath $LastLogPath
        Assert-NoSkillWrites -Root $skillsProject -LogPath $LastLogPath
        if ($RequireSkills) {
            Assert-LogContains -Phase 'install' -Needle "Optional: run 'meshc skills setup'" -LogPath $LastLogPath
            if (Select-String -LiteralPath $LastLogPath -SimpleMatch 'Set up the Mesh skill for your coding tools?' -Quiet) {
                Fail-Phase 'install' 'installation prompted with redirected stdin' $LastLogPath
            }
        }

        $env:USERPROFILE = $goodHome
        Invoke-LoggedCommand -Phase 'install' -Label '04-install-good' -Display 'pwsh -File website/docs/public/install.ps1 -Yes' -Command {
            pwsh -NoProfile -File $script:InstallScript -Yes
        }
    } finally {
        Pop-Location
    }

    $installedMeshc = Join-Path $goodHome '.mesh/bin/meshc.exe'
    $installedMeshpkg = Join-Path $goodHome '.mesh/bin/meshpkg.exe'
    $installedVersion = Join-Path $goodHome '.mesh/version'
    if (-not (Test-Path $installedMeshc)) { Fail-Phase 'install' 'installed meshc.exe was missing' $LastLogPath }
    if (-not (Test-Path $installedMeshpkg)) { Fail-Phase 'install' 'installed meshpkg.exe was missing' $LastLogPath }
    foreach ($lib in @('mesh_rt.lib', 'mesh_test_rt.lib')) {
        if (-not (Test-Path (Join-Path $goodHome ".mesh/lib/$lib"))) { Fail-Phase 'install' "installed $lib was missing" $LastLogPath }
    }
    if (-not (Test-Path $installedVersion)) { Fail-Phase 'install' 'version file was not written' $LastLogPath }
    if ((Get-Content $installedVersion -Raw).Trim() -ne $Version) { Fail-Phase 'install' 'version file did not match staged version' $LastLogPath }
    Assert-NoSkillWrites -Root $goodHome -LogPath $LastLogPath
    Assert-NoSkillWrites -Root $skillsProject -LogPath $LastLogPath
    if ($RequireSkills) {
        Assert-LogContains -Phase 'install' -Needle "Optional: run 'meshc skills setup'" -LogPath $LastLogPath
    }

    Invoke-LoggedCommand -Phase 'version' -Label '05-meshc-version' -Display 'installed meshc.exe --version' -Command {
        & $installedMeshc --version
    }
    if ((Get-Content $LastStdoutPath -Raw).TrimEnd("`r", "`n") -cne "meshc $Version") {
        Fail-Phase 'version' "installed meshc version did not match $Version" $LastLogPath
    }

    Invoke-LoggedCommand -Phase 'version' -Label '06-meshpkg-version' -Display 'installed meshpkg.exe --version' -Command {
        & $installedMeshpkg --version
    }
    if ((Get-Content $LastStdoutPath -Raw).TrimEnd("`r", "`n") -cne "meshpkg $Version") {
        Fail-Phase 'version' "installed meshpkg version did not match $Version" $LastLogPath
    }

    Test-TamperedReleaseArchives -ServerUrl $serverUrl
    if ($RequireSkills) {
        Test-SkillUpgrade -InstallHome $goodHome -Project $skillsProject
    }
    if ($InteractiveSkills) {
        $interactiveProject = Join-Path $WorkRoot 'interactive-project'
        New-Item -ItemType Directory -Path $interactiveProject -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $FixtureDir 'mesh.toml') -Destination $interactiveProject
        Test-InteractiveSkills -Project $interactiveProject -ServerUrl $serverUrl
        $env:USERPROFILE = $goodHome
    }

    $smokeDir = Join-Path $WorkRoot 'installer-smoke'
    New-Item -ItemType Directory -Path $smokeDir -Force | Out-Null
    Copy-Item (Join-Path $FixtureDir 'mesh.toml') (Join-Path $smokeDir 'mesh.toml') -Force
    Copy-Item (Join-Path $FixtureDir 'main.mpl') (Join-Path $smokeDir 'main.mpl') -Force
    $helloExe = Join-Path $RunDir 'installer-smoke.exe'
    $helloTrace = Join-Path $RunDir '07-hello-build.trace.json'
    $helloContext = Join-Path $RunDir '07-hello-build.context.log'
    $helloSummary = Join-Path $RunDir 'diagnostic-summary.json'

    Push-InstalledBuildEnvironment
    try {
        Write-InstalledBuildContextLog -Path $helloContext -InstalledMeshcPath $installedMeshc -InstalledMeshpkgPath $installedMeshpkg -TracePath $helloTrace -HelloExePath $helloExe
        Invoke-InstalledBuildCommand -Phase 'build' -Label '07-hello-build' -Display 'installed meshc.exe build installer smoke fixture' -Command {
            & $installedMeshc build $smokeDir --output $helloExe --no-color
        } -TracePath $helloTrace -SummaryPath $helloSummary -BuildContextLogPath $helloContext -InstalledMeshcPath $installedMeshc -InstalledMeshpkgPath $installedMeshpkg -HostedLogPath $HostedHelloBuildLog
        if ($RequireSkills) {
            $trace = Get-BuildTraceInfo -TracePath $helloTrace
            $installedRuntime = Join-Path $goodHome '.mesh/lib/mesh_rt.lib'
            if (-not $trace.valid -or -not (Test-ObjectProperty -Object $trace.data -Name 'runtimeLibraryPath') -or
                ($trace.data.runtimeLibraryPath -replace '^\\\\\?\\', '') -ne $installedRuntime) {
                Fail-Phase 'build' "the installed build did not report linking $installedRuntime" $LastLogPath
            }
        }
    } finally {
        Pop-InstalledBuildEnvironment
    }

    Invoke-LoggedCommand -Phase 'runtime' -Label '08-hello-run' -Display 'run installed hello binary' -Command {
        & $helloExe
    }
    if ((Get-Content $LastStdoutPath -Raw).TrimEnd("`r", "`n") -cne 'hello') {
        Fail-Phase 'runtime' 'installer smoke binary printed unexpected output' $LastLogPath
    }

} finally {
    Stop-LocalServer
    $env:USERPROFILE = $originalProfile
    # USERPROFILE isolates files. The installer also writes the real HKCU PATH.
    if (-not $userEnvironment) {
        $userEnvironment = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    }
    if ($userEnvironment) {
        try {
            $currentPath = $userEnvironment.GetValue('Path', $null, $pathOptions)
            if ($currentPath) {
                $proofBins = @('good', 'no-console', 'accept', 'decline' | ForEach-Object { Join-Path $HomeRoot "$_\.mesh\bin" })
                $restoredPath = ($currentPath -split ';' | Where-Object { $_ -notin $proofBins }) -join ';'
                if ($restoredPath -cne $currentPath) {
                    if ($restoredPath -ceq [string]$originalReadPath) {
                        $restoredPath = $originalUserPath
                    }
                    if ($null -ne $restoredPath) {
                        $userEnvironment.SetValue('Path', $restoredPath, $originalPathKind)
                    } else {
                        $userEnvironment.DeleteValue('Path', $false)
                    }
                }
            }
        } finally {
            $userEnvironment.Dispose()
        }
    }
}
Write-Host 'verify-m034-s03.ps1: ok'
