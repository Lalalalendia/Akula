# VIDEO-18b-02: real Windows DACL/standard-user negative lab, not an installer.
# Host-only disposable fixtures. NEVER operate on real Akula or Aquilum roots.
#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request') {
    throw 'VIDEO18B lab requires a hosted Windows pull-request runner'
}
$head = (git rev-parse HEAD).Trim()
if (!$env:EXPECTED_HEAD_SHA -or $head -cne $env:EXPECTED_HEAD_SHA) {
    throw 'VIDEO18B lab requires checkout of exact pull-request HEAD'
}
$who = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($who)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'VIDEO18B ACL lab must provision the fixture with an elevated runner'
}
if (!(Get-Command New-LocalUser -ErrorAction SilentlyContinue)) {
    throw 'VIDEO18B lab cannot create a disposable non-admin Windows account'
}
if (!$env:ProgramFiles -or !$env:PUBLIC) {
    throw 'VIDEO18B lab lacks OS-known ProgramFiles or Public directory'
}
$programFiles = [IO.Path]::GetFullPath($env:ProgramFiles)
$tag = [guid]::NewGuid().ToString('N')
$root = [IO.Path]::GetFullPath((Join-Path $programFiles ('AkulaVideo18bLab-' + $tag)))
$media = [IO.Path]::GetFullPath((Join-Path (Join-Path $env:PUBLIC 'Documents') ('AkulaVideo18bMedia-' + $tag)))
if (!$root.StartsWith($programFiles + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
    ([IO.Path]::GetFileName($root) -cne ('AkulaVideo18bLab-' + $tag)) -or
    !([IO.Path]::GetFileName($media) -ceq ('AkulaVideo18bMedia-' + $tag))) {
    throw 'VIDEO18B fixture root is not constrained to a fresh isolated lab'
}
$accountName = 'akv18b' + $tag.Substring(0,10)
$password = 'V18b!' + [guid]::NewGuid().ToString('N') + 'Az9'
$createdAccount = $false
$createdRoot = $false
$createdMedia = $false

function Invoke-AclCommand([string]$Path, [string[]]$Arguments) {
    & icacls.exe $Path @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "VIDEO18B icacls rejected fixture ACL operation" }
}
function Assert-NotReparse([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'VIDEO18B fixture contains a junction or reparse point'
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

public static class AkulaVideo18bAclProbe {
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true, EntryPoint="LogonUserW")]
    static extern bool LogonUser(string username, string domain, string password,
        int logonType, int logonProvider, out SafeAccessTokenHandle token);

    static void ExpectDenied(string label, Action operation) {
        bool denied = false;
        try { operation(); }
        catch (UnauthorizedAccessException) { denied = true; }
        if (!denied)
            throw new InvalidOperationException("VIDEO18B standard-user mutation unexpectedly permitted: " + label);
    }

    public static string Run(string username, string password, string protectedRoot, string mediaRoot) {
        SafeAccessTokenHandle token;
        if (!LogonUser(username, ".", password, 2, 0, out token))
            throw new Win32Exception(Marshal.GetLastWin32Error(), "VIDEO18B standard-user token unavailable");
        using (token) {
            return WindowsIdentity.RunImpersonated<string>(token, () => {
                using (var identity = WindowsIdentity.GetCurrent()) {
                    if (new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator))
                        throw new InvalidOperationException("VIDEO18B attacker unexpectedly has administrator rights");
                }
                string[] names = {
                    "akula-app.exe", "shiva-pdf-preview.exe", "akula-video-mux.exe",
                    "akula-yt-dlp.exe", "akula-quickjs.exe"
                };
                int denied = 0;
                foreach (string name in names) {
                    string file = Path.Combine(protectedRoot, name);
                    if (File.ReadAllText(file, Encoding.UTF8) != "VIDEO18B-LAB-ONLY:" + name)
                        throw new InvalidOperationException("VIDEO18B standard user cannot read an intact sidecar");
                    ExpectDenied("overwrite:" + name, () => File.WriteAllText(file, "ATTACK", Encoding.UTF8));
                    denied++;
                }
                string ytdlp = Path.Combine(protectedRoot, "akula-yt-dlp.exe");
                ExpectDenied("delete", () => File.Delete(ytdlp)); denied++;
                ExpectDenied("rename", () => File.Move(ytdlp, ytdlp + ".renamed")); denied++;
                ExpectDenied("inject helper", () => File.WriteAllText(
                    Path.Combine(protectedRoot, "untrusted-helper.exe"), "ATTACK")); denied++;
                ExpectDenied("replace directory", () => Directory.Move(
                    protectedRoot, protectedRoot + "-swapped")); denied++;
                ExpectDenied("create junction staging directory", () => Directory.CreateDirectory(
                    Path.Combine(protectedRoot, "junction-staging"))); denied++;
                string media = Path.Combine(mediaRoot, "downloaded-local-media.txt");
                File.WriteAllText(media, "LOCAL-ONLY-MEDIA", Encoding.UTF8);
                if (File.ReadAllText(media, Encoding.UTF8) != "LOCAL-ONLY-MEDIA")
                    throw new InvalidOperationException("VIDEO18B local media cannot be saved by standard user");
                return "VIDEO18B_PROTECTED_ROOT_STANDARD_USER_PASS blocked_mutations=" +
                    denied + " readable_helpers=5 local_media_writable=1";
            });
        }
    }
}
'@

try {
    New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
    $createdRoot = $true
    Assert-NotReparse $root
    # Remove inherited user-writable ACEs; grant Users RX only, not Modify.
    Invoke-AclCommand $root @('/inheritance:r','/grant:r',
        '*S-1-5-32-544:(OI)(CI)F','*S-1-5-18:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)RX')
    $names = @('akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs')
    $original = @{}
    foreach ($name in $names) {
        $file = Join-Path $root ($name + '.exe')
        [IO.File]::WriteAllText($file, ('VIDEO18B-LAB-ONLY:' + $name + '.exe'), [Text.Encoding]::UTF8)
        $original[$name] = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash
        Assert-NotReparse $file
    }
    Invoke-AclCommand $root @('/setowner','*S-1-5-32-544','/T')
    Invoke-AclCommand $root @('/verify','/T')
    # Media is a disposable writable control, NOT the real user's library.
    New-Item -ItemType Directory -Path $media -ErrorAction Stop | Out-Null
    $createdMedia = $true
    Assert-NotReparse $media
    Invoke-AclCommand $media @('/inheritance:r','/grant:r',
        '*S-1-5-32-544:(OI)(CI)F','*S-1-5-18:(OI)(CI)F','*S-1-5-32-545:(OI)(CI)M')
    $securePassword = ConvertTo-SecureString -String $password -AsPlainText -Force
    New-LocalUser -Name $accountName -Password $securePassword -PasswordNeverExpires -ErrorAction Stop | Out-Null
    $createdAccount = $true
    if ([bool](Get-LocalGroupMember -Group 'Administrators' -ErrorAction Stop |
        Where-Object { $_.Name -match [regex]::Escape($accountName) })) {
        throw 'VIDEO18B fixture user unexpectedly belongs to Administrators'
    }
    Add-Type -TypeDefinition $native -ErrorAction Stop
    $proof = [AkulaVideo18bAclProbe]::Run($accountName, $password, $root, $media)
    foreach ($name in $names) {
        $file = Join-Path $root ($name + '.exe')
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -cne $original[$name]) {
            throw 'VIDEO18B protected executable was altered during an attacker test'
        }
    }
    if ($proof -notmatch 'blocked_mutations=10 readable_helpers=5 local_media_writable=1') {
        throw 'VIDEO18B standard-user proof is incomplete'
    }
    Write-Host $proof
    Write-Host 'VIDEO18B_PROTECTED_ROOT_LAB_PASS five_mock_exe=1 no_public_installer=1'
}
finally {
    $cleanupFailure = $null
    if ($createdAccount) {
        try { Remove-LocalUser -Name $accountName -ErrorAction Stop }
        catch { $cleanupFailure = 'VIDEO18B cannot remove disposable test account' }
    }
    if ($createdRoot) {
        try { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailure = 'VIDEO18B cannot remove disposable protected test root' }
    }
    if ($createdMedia) {
        try { Remove-Item -LiteralPath $media -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailure = 'VIDEO18B cannot remove disposable test media' }
    }
    if ($cleanupFailure) { throw $cleanupFailure }
}
