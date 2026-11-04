param(
    [ValidateSet('all', 'quick', 'fmt', 'fmt-changed', 'clippy', 'core', 'sync', 'native', 'build-native', 'test', 'audit', 'gui')]
    [string]$Step = 'quick',

    [ValidateRange(1, 180)]
    [int]$TimeoutMinutes = 90
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$app = Join-Path $repo 'aquilum-app'
$artifactRoot = Join-Path $repo '.artifacts'
$target = Join-Path $artifactRoot 'cargo-native'
$devCmd = 'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\VsDevCmd.bat'
$runner = Join-Path $PSScriptRoot 'run_instance.ps1'
$binary = Join-Path $target 'debug\aquilum-native.exe'
$script:buildNativeCompletedAt = $null
$runId = Get-Date -Format 'yyyyMMdd-HHmmssfff'
$runRoot = Join-Path $artifactRoot (Join-Path 'one-pc-checks' $runId)
[void](New-Item -ItemType Directory -Path $runRoot -Force)
$startedAt = Get-Date
$gitHeadAtStart = (& git -C $repo rev-parse HEAD).Trim()
$gitBranchAtStart = (& git -C $repo branch --show-current).Trim()
$gitDirtyAtStart = @(& git -C $repo status --porcelain).Count -gt 0
$cargoLockPath = Join-Path $app 'Cargo.lock'
$hostBrokerLockPath = Join-Path $app 'host-broker-lab\Cargo.lock'
$auditCargoHome = Join-Path $artifactRoot 'one-pc-cargo-home'
$cargoLockSha256 = (Get-FileHash -LiteralPath $cargoLockPath -Algorithm SHA256).Hash
$hostBrokerLockSha256 = (Get-FileHash -LiteralPath $hostBrokerLockPath -Algorithm SHA256).Hash

if (-not (Test-Path -LiteralPath $devCmd -PathType Leaf)) {
    throw "Visual Studio developer environment was not found: $devCmd"
}

$results = New-Object 'System.Collections.Generic.List[object]'

function Protect-TestTempRoot {
    param([Parameter(Mandatory = $true)][string]$Path)

    # File Gate custody tests require an owner-restricted Windows DACL. The
    # workspace's .artifacts parent may grant write access to Authenticated
    # Users, so remove inherited ACEs only from this disposable per-step temp
    # directory and grant FullControl to the current user, SYSTEM and admins.
    $userSid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
    if (-not $userSid) { throw 'Could not identify the current Windows user SID.' }

    $acl = Get-Acl -LiteralPath $Path
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($existingRule in @($acl.Access)) {
        [void]$acl.RemoveAccessRuleSpecific($existingRule)
    }

    $inheritance = [System.Security.AccessControl.InheritanceFlags]::ContainerInherit -bor
        [System.Security.AccessControl.InheritanceFlags]::ObjectInherit
    $principals = @(
        $userSid,
        [System.Security.Principal.SecurityIdentifier]::new('S-1-5-18'),
        [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
    )
    foreach ($sid in $principals) {
        $rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
            $sid,
            [System.Security.AccessControl.FileSystemRights]::FullControl,
            $inheritance,
            [System.Security.AccessControl.PropagationFlags]::None,
            [System.Security.AccessControl.AccessControlType]::Allow
        )
        [void]$acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Path -AclObject $acl

    $verified = Get-Acl -LiteralPath $Path
    $actualSids = @(
        $verified.Access | ForEach-Object {
            $_.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value
        } | Sort-Object -Unique
    )
    $expectedSids = @($userSid.Value, 'S-1-5-18', 'S-1-5-32-544') | Sort-Object -Unique
    if ($verified.Access.Count -ne 3 -or
        (Compare-Object -ReferenceObject $expectedSids -DifferenceObject $actualSids).Count -ne 0) {
        throw 'The per-step test temp directory DACL is not restricted to the expected principals.'
    }
}

function Invoke-CargoStep {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$CargoLines
    )

    $stdout = Join-Path $runRoot "$Name.stdout.log"
    $stderr = Join-Path $runRoot "$Name.stderr.log"
    $metricsPath = Join-Path $runRoot "$Name.resources.csv"
    $commandFile = Join-Path $runRoot "$Name.cmd"
    $stepTempRoot = Join-Path $runRoot "$Name-temp"
    [void](New-Item -ItemType Directory -Path $stepTempRoot -Force)
    if ($Name -in @('core', 'sync', 'native', 'test')) {
        try {
            Protect-TestTempRoot -Path $stepTempRoot
        } catch {
            $message = "Could not secure the per-step test temp directory: $($_.Exception.Message)"
            Set-Content -LiteralPath $stdout -Value '' -Encoding UTF8
            Set-Content -LiteralPath $stderr -Value $message -Encoding UTF8
            [pscustomobject]@{
                Timestamp = (Get-Date).ToString('o')
                ElapsedSeconds = 0
                ProcessCount = 0
                WorkingSetMiB = 0
                TotalCpuSeconds = 0
            } | Export-Csv -LiteralPath $metricsPath -NoTypeInformation -Encoding UTF8
            $results.Add([pscustomobject]@{
                Step = $Name
                Status = 'FAIL'
                ExitCode = 1
                ElapsedSeconds = 0
                Stdout = $stdout
                Stderr = $stderr
                Resources = $metricsPath
            })
            Write-Output "$Name FAIL (0 sec); $message"
            return
        }
    }
    $lines = @(
        '@echo off',
        "call `"$devCmd`" -no_logo -arch=x64 -host_arch=x64",
        'if errorlevel 1 exit /b %errorlevel%',
        "set `"CARGO_TARGET_DIR=$target`"",
        'set "CARGO_INCREMENTAL=0"',
        "set `"TEMP=$stepTempRoot`"",
        "set `"TMP=$stepTempRoot`"",
        "cd /d `"$app`""
    ) + $CargoLines + @('exit /b %errorlevel%')
    Set-Content -LiteralPath $commandFile -Value $lines -Encoding ASCII

    $started = Get-Date
    $process = Start-Process -FilePath $env:ComSpec `
        -ArgumentList @('/d', '/c', $commandFile) `
        -WorkingDirectory $repo -NoNewWindow -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $samples = New-Object 'System.Collections.Generic.List[object]'
    $timedOut = $false

    while (-not $process.WaitForExit(1000)) {
        $now = Get-Date
        $buildProcesses = @(Get-Process cargo, rustc, cl, link -ErrorAction SilentlyContinue)
        $cpu = ($buildProcesses | Measure-Object -Property { $_.CPU } -Sum).Sum
        $workingSet = ($buildProcesses | Measure-Object -Property WorkingSet64 -Sum).Sum
        $samples.Add([pscustomobject]@{
            Timestamp = $now.ToString('o')
            ElapsedSeconds = [math]::Round(($now - $started).TotalSeconds, 1)
            ProcessCount = $buildProcesses.Count
            WorkingSetMiB = [math]::Round(($workingSet / 1MB), 1)
            TotalCpuSeconds = [math]::Round($cpu, 1)
        })
        if (($now - $started).TotalMinutes -ge $TimeoutMinutes) {
            $timedOut = $true
            & taskkill.exe /PID $process.Id /T /F | Out-Null
            break
        }
    }

    if (-not $timedOut) { $process.WaitForExit() }
    $elapsed = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    $exitCode = if ($timedOut) { 124 } else { $process.ExitCode }
    if ($Name -eq 'build-native' -and $exitCode -eq 0) {
        $script:buildNativeCompletedAt = Get-Date
    }
    if ($samples.Count -eq 0) {
        $samples.Add([pscustomobject]@{
            Timestamp = (Get-Date).ToString('o')
            ElapsedSeconds = $elapsed
            ProcessCount = 0
            WorkingSetMiB = 0
            TotalCpuSeconds = 0
        })
    }
    $samples | Export-Csv -LiteralPath $metricsPath -NoTypeInformation -Encoding UTF8

    $result = [pscustomobject]@{
        Step = $Name
        Status = if ($timedOut) { 'TIMEOUT' } elseif ($exitCode -eq 0) { 'PASS' } else { 'FAIL' }
        ExitCode = $exitCode
        ElapsedSeconds = $elapsed
        Stdout = $stdout
        Stderr = $stderr
        Resources = $metricsPath
    }
    $results.Add($result)
    Write-Output "$Name $($result.Status) ($elapsed sec); log: $stdout"
}

function Invoke-GuiSmoke {
    $stdout = Join-Path $runRoot 'gui.stdout.log'
    $stderr = Join-Path $runRoot 'gui.stderr.log'
    $metricsPath = Join-Path $runRoot 'gui.resources.csv'
    $metrics = New-Object 'System.Collections.Generic.List[object]'
    $pids = New-Object 'System.Collections.Generic.List[int]'
    $started = Get-Date
    $status = 'FAIL'
    $message = ''

    try {
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
            throw "Native debug binary not found: $binary. Build it first."
        }
        $binaryItem = Get-Item -LiteralPath $binary
        $buildInputs = @(
            Get-Item -LiteralPath @(
                (Join-Path $app 'Cargo.toml'),
                (Join-Path $app 'Cargo.lock'),
                (Join-Path $app 'core\Cargo.toml'),
                (Join-Path $app 'native\Cargo.toml')
            )
            Get-ChildItem -LiteralPath @(
                (Join-Path $app 'core'),
                (Join-Path $app 'native')
            ) -Filter '*.rs' -Recurse -File
        )
        $latestSource = $buildInputs | Sort-Object -Property LastWriteTime -Descending | Select-Object -First 1
        if ($latestSource -and $latestSource.LastWriteTime -gt $binaryItem.LastWriteTime -and
            (-not $script:buildNativeCompletedAt -or $script:buildNativeCompletedAt -lt $latestSource.LastWriteTime)) {
            throw "Native debug binary is older than build input $($latestSource.FullName). Run -Step build-native first."
        }
        $profileRoot = Join-Path $artifactRoot 'one-pc-instances'
        $profileA = Get-Content -LiteralPath (Join-Path $profileRoot 'A\profile.json') -Raw | ConvertFrom-Json
        $profileB = Get-Content -LiteralPath (Join-Path $profileRoot 'B\profile.json') -Raw | ConvertFrom-Json
        if ($profileA.app_data_dir -eq $profileB.app_data_dir -or $profileA.vault_root -eq $profileB.vault_root) {
            throw 'The local smoke profiles do not have distinct app-data and vault directories.'
        }

        foreach ($instance in @('A', 'B')) {
            $launchOutput = (& $runner -Instance $instance -Executable $binary 2>&1 | Out-String)
            Add-Content -LiteralPath $stdout -Value $launchOutput -Encoding UTF8
            $match = [regex]::Match($launchOutput, 'pid=(\d+)')
            if (-not $match.Success) { throw "Could not read process ID for profile $instance. $launchOutput" }
            $pids.Add([int]$match.Groups[1].Value)
        }

        $deadline = (Get-Date).AddSeconds(30)
        $windowsReady = $false
        while ((Get-Date) -lt $deadline) {
            $windows = @($pids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
            if ($windows.Count -eq 2 -and @($windows | Where-Object { $_.MainWindowHandle -eq [IntPtr]::Zero }).Count -eq 0) {
                $windowsReady = $true
                break
            }
            Start-Sleep -Milliseconds 500
        }
        if (-not $windowsReady) { throw 'Both Akula windows did not become ready within 30 seconds.' }

        foreach ($instance in @('A', 'B')) {
            $appData = Join-Path $profileRoot "$instance\appdata"
            foreach ($file in @('ui-state.sqlite3', 'documents.sqlite3', 'device-id')) {
                if (-not (Test-Path -LiteralPath (Join-Path $appData $file) -PathType Leaf)) {
                    throw "Profile $instance did not create its own $file."
                }
            }
        }
        $deviceA = (Get-FileHash -Algorithm SHA256 (Join-Path $profileRoot 'A\appdata\device-id')).Hash
        $deviceB = (Get-FileHash -Algorithm SHA256 (Join-Path $profileRoot 'B\appdata\device-id')).Hash
        if ($deviceA -eq $deviceB) { throw 'The app-data device identifiers unexpectedly match.' }

        $status = 'PASS'
        $message = 'Two separate Akula windows opened isolated app-data/vault profiles; device-id hashes differ. This is not distinct P2P-key or physical-device acceptance.'
    } catch {
        $message = $_.Exception.Message
        Add-Content -LiteralPath $stderr -Value $message -Encoding UTF8
    } finally {
        foreach ($id in $pids) {
            $process = Get-Process -Id $id -ErrorAction SilentlyContinue
            if ($process) { [void]$process.CloseMainWindow() }
        }
        foreach ($id in $pids) {
            try { Wait-Process -Id $id -Timeout 20 -ErrorAction Stop } catch {
                # A window can exit just before Wait-Process starts and make
                # the cmdlet report "not found". Only fail if it remains alive.
                $remaining = Get-Process -Id $id -ErrorAction SilentlyContinue
                if ($remaining) {
                    $status = 'FAIL'
                    $message = "$message Akula process $id did not close gracefully; left running."
                    Add-Content -LiteralPath $stderr -Value "Did not force-terminate process $id." -Encoding UTF8
                }
            }
        }
    }

    $elapsed = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    $metrics.Add([pscustomobject]@{
        Timestamp = (Get-Date).ToString('o')
        ElapsedSeconds = $elapsed
        ProcessIds = ($pids -join ';')
        Status = $status
    })
    $metrics | Export-Csv -LiteralPath $metricsPath -NoTypeInformation -Encoding UTF8
    $result = [pscustomobject]@{
        Step = 'gui'
        Status = $status
        ExitCode = if ($status -eq 'PASS') { 0 } else { 1 }
        ElapsedSeconds = $elapsed
        Message = $message
        Stdout = $stdout
        Stderr = $stderr
        Resources = $metricsPath
    }
    $results.Add($result)
    Write-Output "gui $status ($elapsed sec): $message"
}

function Get-RustEdition {
    param([Parameter(Mandatory = $true)][string]$FilePath)

    $directory = Split-Path -Parent $FilePath
    while ($directory.StartsWith($repo, [System.StringComparison]::OrdinalIgnoreCase)) {
        $manifest = Join-Path $directory 'Cargo.toml'
        if (Test-Path -LiteralPath $manifest -PathType Leaf) {
            $manifestText = Get-Content -LiteralPath $manifest -Raw
            $package = [regex]::Match($manifestText, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
            if ($package.Success) {
                $edition = [regex]::Match($package.Groups[1].Value, '(?m)^\s*edition\s*=\s*"(?<edition>[^"]+)"')
                if ($edition.Success) { return $edition.Groups['edition'].Value }

                if ($package.Groups[1].Value -match '(?m)^\s*edition\.workspace\s*=\s*true') {
                    $workspaceDirectory = $directory
                    while ($workspaceDirectory.StartsWith($repo, [System.StringComparison]::OrdinalIgnoreCase)) {
                        $workspaceManifest = Join-Path $workspaceDirectory 'Cargo.toml'
                        if (Test-Path -LiteralPath $workspaceManifest -PathType Leaf) {
                            $workspaceText = Get-Content -LiteralPath $workspaceManifest -Raw
                            $workspacePackage = [regex]::Match($workspaceText, '(?ms)^\[workspace\.package\]\s*(.*?)(?=^\[|\z)')
                            if ($workspacePackage.Success) {
                                $workspaceEdition = [regex]::Match($workspacePackage.Groups[1].Value, '(?m)^\s*edition\s*=\s*"(?<edition>[^"]+)"')
                                if ($workspaceEdition.Success) { return $workspaceEdition.Groups['edition'].Value }
                            }
                        }
                        $parentWorkspace = Split-Path -Parent $workspaceDirectory
                        if ($parentWorkspace -eq $workspaceDirectory) { break }
                        $workspaceDirectory = $parentWorkspace
                    }
                    throw "Cargo workspace edition was not found for $FilePath."
                }
            }
        }
        $parent = Split-Path -Parent $directory
        if ($parent -eq $directory) { break }
        $directory = $parent
    }

    throw "Cargo package edition was not found for $FilePath."
}

function Invoke-ChangedRustFormat {
    $stdout = Join-Path $runRoot 'fmt-changed.stdout.log'
    $stderr = Join-Path $runRoot 'fmt-changed.stderr.log'
    $metricsPath = Join-Path $runRoot 'fmt-changed.resources.csv'
    $stdoutLines = New-Object 'System.Collections.Generic.List[string]'
    $stderrLines = New-Object 'System.Collections.Generic.List[string]'
    $started = Get-Date
    $failed = 0
    $inheritedDriftFiles = New-Object 'System.Collections.Generic.List[string]'
    $tracked = @(& git -C $repo diff --name-only --diff-filter=ACMR HEAD -- '*.rs')
    if ($LASTEXITCODE -ne 0) { throw 'Could not list tracked Rust changes for formatting.' }
    $untracked = @(& git -C $repo ls-files --others --exclude-standard -- '*.rs')
    if ($LASTEXITCODE -ne 0) { throw 'Could not list untracked Rust files for formatting.' }
    $files = @($tracked + $untracked | Where-Object { $_ -match '\.rs$' } | Sort-Object -Unique)

    if ($files.Count -eq 0) {
        $stdoutLines.Add('No changed or untracked Rust source files.')
    }
    foreach ($relativePath in $files) {
        $fullPath = Join-Path $repo ($relativePath -replace '/', '\\')
        try {
            $edition = Get-RustEdition -FilePath $fullPath
            $stdoutLines.Add("rustfmt +1.96.0 --check --edition $edition --config skip_children=true $relativePath")
            $output = & rustfmt +1.96.0 --check --edition $edition --config skip_children=true $fullPath 2>&1
            $exitCode = $LASTEXITCODE
            if ($exitCode -ne 0) {
                $formatLocations = @(
                    foreach ($line in $output) {
                        if ([string]$line -match '^Diff in .+:(?<line>\d+):') {
                            [int]$Matches['line']
                        }
                    }
                )
                $changedRanges = New-Object 'System.Collections.Generic.List[object]'
                if ($tracked -contains $relativePath) {
                    $fileDiff = @(& git -C $repo diff --unified=0 HEAD -- $relativePath)
                    if ($LASTEXITCODE -ne 0) { throw "Could not find changed lines in $relativePath." }
                    foreach ($hunk in $fileDiff) {
                        if ([string]$hunk -match '^@@ -\d+(?:,\d+)? \+(?<start>\d+)(?:,(?<count>\d+))? @@') {
                            $startLine = [int]$Matches['start']
                            $countText = [string]$Matches['count']
                            $changedCount = if ([string]::IsNullOrEmpty($countText)) { 1 } else { [int]$countText }
                            if ($changedCount -gt 0) {
                                $changedRanges.Add([pscustomobject]@{
                                    Start = $startLine
                                    End = $startLine + $changedCount - 1
                                })
                            }
                        }
                    }
                }

                $formatTouchesChangedLines = $false
                foreach ($location in $formatLocations) {
                    foreach ($range in $changedRanges) {
                        if ($location -ge ($range.Start - 2) -and $location -le ($range.End + 2)) {
                            $formatTouchesChangedLines = $true
                        }
                    }
                }
                if ($formatLocations.Count -gt 0 -and $changedRanges.Count -gt 0 -and -not $formatTouchesChangedLines) {
                    $stdoutLines.Add("Inherited rustfmt drift outside changed lines: $relativePath")
                    $inheritedDriftFiles.Add($relativePath)
                    foreach ($line in $output) { $stderrLines.Add([string]$line) }
                } else {
                    foreach ($line in $output) { $stderrLines.Add([string]$line) }
                    $stderrLines.Add("rustfmt exited with code $exitCode for $relativePath")
                    $failed++
                }
            } else {
                foreach ($line in $output) { $stdoutLines.Add([string]$line) }
            }
        } catch {
            $stderrLines.Add($relativePath + [char]9 + $_.Exception.Message)
            $failed++
        }
    }

    $stdoutLines | Set-Content -LiteralPath $stdout -Encoding UTF8
    $stderrLines | Set-Content -LiteralPath $stderr -Encoding UTF8
    $elapsed = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
    [pscustomobject]@{
        Timestamp = (Get-Date).ToString('o')
        ElapsedSeconds = $elapsed
        ProcessCount = 0
        WorkingSetMiB = 0
        TotalCpuSeconds = 0
    } | Export-Csv -LiteralPath $metricsPath -NoTypeInformation -Encoding UTF8
    $status = if ($failed -eq 0) { 'PASS' } else { 'FAIL' }
    $results.Add([pscustomobject]@{
        Step = 'fmt-changed'
        Status = $status
        ExitCode = if ($failed -eq 0) { 0 } else { 1 }
        ElapsedSeconds = $elapsed
        CheckedRustFiles = $files.Count
        FailedRustFiles = $failed
        InheritedDriftFiles = @($inheritedDriftFiles.ToArray())
        Stdout = $stdout
        Stderr = $stderr
        Resources = $metricsPath
    })
    Write-Output "fmt-changed $status ($elapsed sec); checked $($files.Count) Rust files; log: $stdout"
}

$stepsToRun = switch ($Step) {
    'all' { @('fmt', 'clippy', 'test', 'audit', 'build-native', 'gui'); break }
    'quick' { @('fmt-changed', 'core', 'build-native', 'gui'); break }
    default { @($Step) }
}
foreach ($name in $stepsToRun) {
    switch ($name) {
        'fmt' {
            Invoke-CargoStep -Name 'fmt' -CargoLines @('cargo +1.96.0 fmt --all -- --check')
        }
        'fmt-changed' { Invoke-ChangedRustFormat }
        'clippy' {
            Invoke-CargoStep -Name 'clippy' -CargoLines @('cargo +1.96.0 clippy --workspace --all-targets --locked')
        }
        'core' {
            Invoke-CargoStep -Name 'core' -CargoLines @('cargo +1.96.0 test --locked -p aquilum-core')
        }
        'sync' {
            Invoke-CargoStep -Name 'sync' -CargoLines @('cargo +1.96.0 test --locked -p aquilum-core --features sync-engine')
        }
        'native' {
            Invoke-CargoStep -Name 'native' -CargoLines @('cargo +1.96.0 test --locked -p aquilum-native -- --skip app::tests::perf::frames::')
        }
        'build-native' {
            Invoke-CargoStep -Name 'build-native' -CargoLines @('cargo +1.96.0 build --offline --locked -p aquilum-native')
        }
        'test' {
            Invoke-CargoStep -Name 'test' -CargoLines @(
                'cargo +1.96.0 test --workspace --locked -- --skip app::tests::perf::frames::',
                'if errorlevel 1 exit /b %errorlevel%',
                'cargo +1.96.0 test --locked -p aquilum-core --features sync-engine'
            )
        }
        'audit' {
            Invoke-CargoStep -Name 'audit' -CargoLines @(
                "set `"CARGO_HOME=$auditCargoHome`"",
                'cargo audit --no-yanked --file Cargo.lock',
                'if errorlevel 1 exit /b %errorlevel%',
                'cargo audit --no-yanked --no-fetch --file host-broker-lab\Cargo.lock'
            )
        }
        'gui' { Invoke-GuiSmoke }
    }
}

$summary = [pscustomobject]@{
    RunId = $runId
    StartedAt = $startedAt.ToString('o')
    CompletedAt = (Get-Date).ToString('o')
    GitBranchAtStart = $gitBranchAtStart
    GitHeadAtStart = $gitHeadAtStart
    GitHeadAtEnd = (& git -C $repo rev-parse HEAD).Trim()
    GitDirtyAtStart = $gitDirtyAtStart
    CargoLockSha256 = $cargoLockSha256
    HostBrokerLockSha256 = $hostBrokerLockSha256
    RustToolchain = '1.96.0-x86_64-pc-windows-msvc'
    NotGithubMainGate = $true
    Results = $results.ToArray()
}
$summaryPath = Join-Path $runRoot 'summary.json'
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
Write-Output "summary=$summaryPath"
if (@($results | Where-Object { $_.Status -ne 'PASS' }).Count -gt 0) {
    throw "One or more local steps failed. Inspect retained logs under $runRoot."
}
