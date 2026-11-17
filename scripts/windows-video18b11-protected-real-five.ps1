# VIDEO-18b-11: real five-PE protected unsigned NSIS installation and denial lab.
# Hosted PR-only disposable fixtures; never operate on production Akula or user libraries.
#requires -Version 7.2
param(
    [Parameter(Mandatory=$true)][string]$AppPath,
    [Parameter(Mandatory=$true)][string]$ShivaPath,
    [Parameter(Mandatory=$true)][string]$MuxPath,
    [Parameter(Mandatory=$true)][string]$YtDlpPath,
    [Parameter(Mandatory=$true)][string]$QuickJsPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request') {
    throw 'VIDEO18B11 lab requires a hosted Windows pull-request runner'
}
$head = (git rev-parse HEAD).Trim()
if (!$env:EXPECTED_HEAD_SHA -or $head -cne $env:EXPECTED_HEAD_SHA) {
    throw 'VIDEO18B11 lab requires checkout of exact pull-request HEAD'
}
$who = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($who)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'VIDEO18B11 ACL lab must provision the fixture with an elevated runner'
}
if (!(Get-Command New-LocalUser -ErrorAction SilentlyContinue)) {
    throw 'VIDEO18B11 lab cannot create a disposable non-admin Windows account'
}
if (!$env:ProgramFiles -or !$env:PUBLIC) {
    throw 'VIDEO18B11 lab lacks OS-known ProgramFiles or Public directory'
}
$programFiles = [IO.Path]::GetFullPath($env:ProgramFiles)
$tag = [guid]::NewGuid().ToString('N')
$root = [IO.Path]::GetFullPath((Join-Path $programFiles ('AkulaVideo18bRealNsisLab-' + $tag)))
$media = [IO.Path]::GetFullPath((Join-Path (Join-Path $env:PUBLIC 'Documents') ('AkulaVideo18bRealNsisMedia-' + $tag)))
if (!$root.StartsWith($programFiles + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
    ([IO.Path]::GetFileName($root) -cne ('AkulaVideo18bRealNsisLab-' + $tag)) -or
    !([IO.Path]::GetFileName($media) -ceq ('AkulaVideo18bRealNsisMedia-' + $tag))) {
    throw 'VIDEO18B11 fixture root is not constrained to a fresh isolated lab'
}
$accountName = 'akv18b' + $tag.Substring(0,10)
$password = 'V18b!' + [guid]::NewGuid().ToString('N') + 'Az9'
$createdAccount = $false
$createdRoot = $false
$createdMedia = $false
$createdStage = $false
$uninstalled = $false
$stage = Join-Path $env:RUNNER_TEMP ('Video18bRealNsisStage-' + $tag)
$setup = Join-Path $stage 'isolated-real-unsigned-setup.exe'
$nsi = Join-Path $env:GITHUB_WORKSPACE 'scripts/video18b-protected-five-real.nsi'
$mk = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'NSIS\makensis.exe'
if (!(Test-Path -LiteralPath $mk -PathType Leaf) -or
    !(Test-Path -LiteralPath $nsi -PathType Leaf) -or
    (Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $stage)) {
    throw 'VIDEO18B11 compiler, NSIS source or clean fixture missing'
}

function Invoke-AclCommand([string]$Path, [string[]]$Arguments) {
    & icacls.exe $Path @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "VIDEO18B11 icacls rejected fixture ACL operation" }
}
function Assert-NotReparse([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'VIDEO18B11 fixture contains a junction or reparse point'
    }
}

$native = @'
using System;
using System.IO;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class AkulaVideo18bRealAclProbe {
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true, EntryPoint="LogonUserW")]
    static extern bool LogonUser(string username, string domain, string password,
        int logonType, int logonProvider, out SafeAccessTokenHandle token);

    static void ExpectDenied(string label, Action operation) {
        bool denied = false;
        try { operation(); }
        catch (UnauthorizedAccessException) { denied = true; }
        catch (IOException ex) when (ex.HResult == unchecked((int)0x80070005)) {
            // Some Win32 directory operations surface ERROR_ACCESS_DENIED as IOException.
            denied = true;
        }
        if (denied) Console.WriteLine("VIDEO18B11_EXACT_ACCESS_DENIED " + label);
        if (!denied)
            throw new InvalidOperationException("VIDEO18B11 standard-user mutation unexpectedly permitted: " + label);
    }

    public static string Run(string username, string password, string protectedRoot, string mediaRoot) {
        SafeAccessTokenHandle token;
        if (!LogonUser(username, ".", password, 2, 0, out token))
            throw new Win32Exception(Marshal.GetLastWin32Error(), "VIDEO18B11 standard-user token unavailable");
        using (token) {
            return WindowsIdentity.RunImpersonated<string>(token, () => {
                using (var identity = WindowsIdentity.GetCurrent()) {
                    if (new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator))
                        throw new InvalidOperationException("VIDEO18B11 attacker unexpectedly has administrator rights");
                }
                string[] names = {
                    "akula-app.exe", "shiva-pdf-preview.exe", "akula-video-mux.exe",
                    "akula-yt-dlp.exe", "akula-quickjs.exe"
                };
                int denied = 0;
                foreach (string name in names) {
                    string file = Path.Combine(protectedRoot, name);
                    if (!File.Exists(file) || new FileInfo(file).Length < 512)
                        throw new InvalidOperationException("VIDEO18B11 standard user cannot read a genuine PE helper");
                    using (var stream = File.OpenRead(file)) {
                        if (stream.ReadByte() != 77 || stream.ReadByte() != 90)
                            throw new InvalidOperationException("VIDEO18B11 helper lost its PE/MZ signature");
                    }
                    ExpectDenied("overwrite:" + name, () => File.WriteAllText(file, "ATTACK", Encoding.UTF8));
                    denied++;
                }
                Console.WriteLine("VIDEO18B11_FIVE_HELPER_READ_AND_OVERWRITE_DENIAL_PASS");
                string ytdlp = Path.Combine(protectedRoot, "akula-yt-dlp.exe");
                ExpectDenied("delete", () => File.Delete(ytdlp)); denied++;
                ExpectDenied("rename", () => File.Move(ytdlp, ytdlp + ".renamed")); denied++;
                ExpectDenied("inject helper", () => File.WriteAllText(
                    Path.Combine(protectedRoot, "untrusted-helper.exe"), "ATTACK")); denied++;
                ExpectDenied("replace directory", () => Directory.Move(
                    protectedRoot, protectedRoot + "-swapped")); denied++;
                ExpectDenied("create junction staging directory", () => Directory.CreateDirectory(
                    Path.Combine(protectedRoot, "junction-staging"))); denied++;
                ExpectDenied("uninstaller overwrite", () => File.WriteAllText(
                    Path.Combine(protectedRoot, "uninstall.exe"), "ATTACK", Encoding.UTF8));
                denied++;
                string media = Path.Combine(mediaRoot, "downloaded-local-media.txt");
                File.WriteAllText(media, "LOCAL-ONLY-MEDIA", Encoding.UTF8);
                if (File.ReadAllText(media, Encoding.UTF8) != "LOCAL-ONLY-MEDIA")
                    throw new InvalidOperationException("VIDEO18B11 local media cannot be saved by standard user");
                return "VIDEO18B11_PROTECTED_ROOT_STANDARD_USER_PASS blocked_mutations=" +
                    denied + " readable_helpers=5 local_media_writable=1";
            });
        }
    }
}
'@

try {
    New-Item -ItemType Directory -Path $stage -ErrorAction Stop | Out-Null
    $createdStage = $true
    $names = @('akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs')
    $sources = [ordered]@{
        'akula-app' = $AppPath
        'shiva-pdf-preview' = $ShivaPath
        'akula-video-mux' = $MuxPath
        'akula-yt-dlp' = $YtDlpPath
        'akula-quickjs' = $QuickJsPath
    }
    $allowPath = Join-Path $env:GITHUB_WORKSPACE 'docs/release/video18b04-five-exe-allowlist.json'
    $allow = Get-Content -LiteralPath $allowPath -Raw | ConvertFrom-Json
    if ($allow.release_admitted -ne $false -or
        $allow.installer_signing_authorized -ne $false -or
        $allow.stage_binary_distribution_authorized -ne $false -or
        @($allow.components).Count -ne 5 -or
        (@($allow.components | ForEach-Object { $_.name }) -join ',') -cne
            (($names | ForEach-Object { $_ + '.exe' }) -join ',')) {
        throw 'VIDEO18B11 lab must preserve the non-release five-EXE source boundary'
    }
    $allowed = @(
        ([IO.Path]::GetFullPath($env:GITHUB_WORKSPACE).TrimEnd('\\') + '\\')
        ([IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\\') + '\\')
    )
    $original = @{}
    foreach ($name in $names) {
        $source = [IO.Path]::GetFullPath($sources[$name])
        if (!($allowed | Where-Object {
            $source.StartsWith($_, [StringComparison]::OrdinalIgnoreCase)
        })) { throw "VIDEO18B11 source outside hosted CI roots: $name" }
        if (!(Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "VIDEO18B11 real source missing: $name"
        }
        Assert-NotReparse $source
        $stream = [IO.File]::OpenRead($source)
        try {
            if ($stream.Length -lt 512 -or $stream.ReadByte() -ne 77 -or
                $stream.ReadByte() -ne 90) {
                throw "VIDEO18B11 source is not an actual PE/MZ executable: $name"
            }
        } finally { $stream.Dispose() }
        $file = Join-Path $stage ($name + '.exe')
        Copy-Item -LiteralPath $source -Destination $file -ErrorAction Stop
        Assert-NotReparse $file
        $expected = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        $actual = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expected -cne $actual) { throw "VIDEO18B11 source/stage mismatch: $name" }
        $original[$name] = $actual
        if ($name -ceq 'akula-video-mux' -and
            (!$env:AKULA_BUNDLED_MUX_SHA256 -or
             $actual -cne $env:AKULA_BUNDLED_MUX_SHA256)) {
            throw 'VIDEO18B11 Rust mux source hash does not match compiled Akula pin'
        }
        if ($name -in @('akula-yt-dlp','akula-quickjs')) {
            $entry = @($allow.components | Where-Object { $_.name -ceq ($name + '.exe') })
            if ($entry.Count -ne 1 -or $actual -cne $entry[0].sha256 -or
                (Get-Item -LiteralPath $file).Length -ne $entry[0].size_bytes) {
                throw "VIDEO18B11 upstream helper source hash or size untrusted: $name"
            }
        }
    }
    if (@(Get-ChildItem -LiteralPath $stage -File).Count -ne 5) {
        throw 'VIDEO18B11 stage does not contain exactly five real EXEs'
    }
    & $mk "/DLAB_ID=$tag" "/DLAB_STAGE=$stage" "/DLAB_OUTPUT=$setup" $nsi | Out-Host
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $setup -PathType Leaf)) {
        throw 'VIDEO18B11 compilation of genuine NSIS failed'
    }
    $install = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
    if (Test-Path -LiteralPath $root -PathType Container) { $createdRoot = $true }
    if ($install.ExitCode -ne 0 -or !$createdRoot) {
        throw 'VIDEO18B11 protected NSIS installation failed'
    }
    Assert-NotReparse $root
    foreach ($name in $names) {
        $file = Join-Path $root ($name + '.exe')
        if (!(Test-Path -LiteralPath $file -PathType Leaf)) {
            throw 'VIDEO18B11 NSIS did not install all five genuine binaries'
        }
        Assert-NotReparse $file
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $original[$name]) {
            throw 'VIDEO18B11 NSIS changed installed real EXE SHA256'
        }
    }
    $uninstaller = Join-Path $root 'uninstall.exe'
    if (!(Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        throw 'VIDEO18B11 genuine NSIS uninstaller missing'
    }
    Write-Host 'VIDEO18B11_GENUINE_PROTECTED_REAL_NSIS_INSTALL_PASS program_files=1 real_exe_sha256=5 signed=0'
    # Media is a disposable writable control, NOT the real user's library.
    New-Item -ItemType Directory -Path $media -ErrorAction Stop | Out-Null
    $createdMedia = $true
    Assert-NotReparse $media
    Invoke-AclCommand $media @('/inheritance:r','/grant:r',
        '*S-1-5-32-544:(OI)(CI)F','*S-1-5-18:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)M')
    $securePassword = ConvertTo-SecureString -String $password -AsPlainText -Force
    New-LocalUser -Name $accountName -Password $securePassword -PasswordNeverExpires -ErrorAction Stop | Out-Null
    $createdAccount = $true
    # Local account creation may not add Users explicitly on every hosted image.
    $usersSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-32-545')
    $usersMembers = @(Get-LocalGroupMember -SID $usersSid -ErrorAction Stop)
    if (@($usersMembers | Where-Object { $_.Name -match [regex]::Escape($accountName) }).Count -eq 0) {
        Add-LocalGroupMember -SID $usersSid -Member $accountName -ErrorAction Stop
    }
    if ([bool](Get-LocalGroupMember -Group 'Administrators' -ErrorAction Stop |
        Where-Object { $_.Name -match [regex]::Escape($accountName) })) {
        throw 'VIDEO18B11 fixture user unexpectedly belongs to Administrators'
    }
    Add-Type -TypeDefinition $native -ErrorAction Stop
    $proof = [AkulaVideo18bRealAclProbe]::Run($accountName, $password, $root, $media)
    foreach ($name in $names) {
        $file = Join-Path $root ($name + '.exe')
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $original[$name]) {
            throw 'VIDEO18B11 protected executable was altered during an attacker test'
        }
    }
    if ($proof -notmatch 'blocked_mutations=11 readable_helpers=5 local_media_writable=1') {
        throw 'VIDEO18B11 standard-user proof is incomplete'
    }
    Write-Host $proof
    $removed = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
    if ($removed.ExitCode -ne 0) { throw 'VIDEO18B11 NSIS uninstall failed' }
    $uninstalled = $true
    if (Test-Path -LiteralPath $root) { throw 'VIDEO18B11 NSIS uninstall left protected root' }
    if (!(Test-Path -LiteralPath (Join-Path $media 'downloaded-local-media.txt'))) {
        throw 'VIDEO18B11 NSIS uninstaller removed synthetic media'
    }
    Write-Host 'VIDEO18B11_GENUINE_NSIS_UNINSTALL_PASS protected_root_removed=1 media_preserved=1'
    Write-Host 'VIDEO18B11_PROTECTED_REAL_FIVE_NSIS_PASS real_exe=5 blocked_mutations=11 no_release=1'
}
finally {
    $cleanupFailure = $null
    if ($createdAccount) {
        try { Remove-LocalUser -Name $accountName -ErrorAction Stop }
        catch { $cleanupFailure = 'VIDEO18B11 cannot remove disposable test account' }
    }
    if ($createdRoot -and !$uninstalled -and (Test-Path -LiteralPath $root)) {
        try {
            Assert-NotReparse $root
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop
        }
        catch { $cleanupFailure = 'VIDEO18B11 cannot remove disposable protected test root' }
    }
    if ($createdStage) {
        try { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailure = 'VIDEO18B11 cannot remove isolated NSIS staging' }
    }
    if ($createdMedia) {
        try { Remove-Item -LiteralPath $media -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailure = 'VIDEO18B11 cannot remove disposable test media' }
    }
    if ($cleanupFailure) { throw $cleanupFailure }
}
