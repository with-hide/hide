# A Windows device's start script (hide-node/src/ssh/bootstrap.rs): the same
# steps, answers and lock step as the POSIX script there, in the Windows
# PowerShell 5.1 every Windows 10 and 11 carries. The wrapper the ssh command
# starts has read this script's own line and nothing past it: $i is its
# standard input, unbuffered, and $o its standard output, both the link.
#
# Every answer is written as bytes to $o, never through PowerShell's own
# output, which would re-encode it and add its own lines; nothing a statement
# returns may reach the pipeline, so every call that returns a value it does
# not need is cast to [void].

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$stdin = $i
$stdout = $o
$utf8 = New-Object Text.UTF8Encoding $false
$sha = [Security.Cryptography.SHA256]::Create()

# What arrived and is not read yet, so a line and the bytes after it come
# from one place. The node sends nothing past a step until it is answered,
# so nothing is left here when a step hands the link to another program.
$buffer = New-Object byte[] 65536
$state = @{ At = 0; End = 0; Root = $null; Pending = $null }

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$user = $identity.User.Value
$tokenOwner = $identity.Owner.Value
# The platform's own rule (hide_platform::fs::private): this account, the
# system and the administrators may change a folder that holds code the
# account runs; nobody else.
$trusted = @($user, $tokenOwner, 'S-1-5-18', 'S-1-5-32-544')
# Owners of system folders, under which others may add entries of their own
# but not touch what is there (the sticky rule): the system, the
# administrators and TrustedInstaller, which owns a system drive's root.
$systemOwners = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
# Access that changes a file or folder: write, append, write its extended
# attributes or attributes, delete a child, delete, change the access list
# or the owner, and the generic forms of those.
$change = 0x500D0156
# Adding a file or a folder of one's own to a folder.
$addEntry = 0x6

function Fill {
    $state.At = 0
    $state.End = $stdin.Read($buffer, 0, $buffer.Length)
    if ($state.End -le 0) { throw (New-Object IO.EndOfStreamException) }
}

function Read-Line {
    $line = New-Object IO.MemoryStream
    while ($true) {
        if ($state.At -ge $state.End) { Fill }
        $end = [Array]::IndexOf($buffer, [byte]10, $state.At, $state.End - $state.At)
        if ($end -ge 0) {
            $line.Write($buffer, $state.At, $end - $state.At)
            $state.At = $end + 1
            return $utf8.GetString($line.ToArray())
        }
        $line.Write($buffer, $state.At, $state.End - $state.At)
        $state.At = $state.End
    }
}

# Exactly $count bytes of the link, written to $sink while it takes them; a
# sink that fails stops being written to and the bytes are still read, so the
# link stays in step. Answers whether the sink took them all.
function Copy-Bytes([long]$count, $sink) {
    $whole = $true
    while ($count -gt 0) {
        if ($state.At -ge $state.End) { Fill }
        $n = [int][Math]::Min([long]($state.End - $state.At), $count)
        if ($null -ne $sink -and $whole) {
            try { $sink.Write($buffer, $state.At, $n) } catch { $whole = $false }
        }
        $state.At += $n
        $count -= $n
    }
    $whole
}

function Say {
    $text = 'HIDE1'
    foreach ($word in $args) { $text += "`t" + ([string]$word -replace '[\x00-\x1f\x7f]', ' ') }
    $bytes = $utf8.GetBytes($text + "`n")
    $stdout.Write($bytes, 0, $bytes.Length)
    $stdout.Flush()
}

# A path as the node spells it (`C:/Users/me`) and as this system does.
function Native([string]$wire) { $wire.Replace('/', '\') }
function Wire([string]$native) {
    $wire = $native.Replace('\', '/')
    if ($wire -match '^[a-z]:') { $wire = $wire.Substring(0, 1).ToUpperInvariant() + $wire.Substring(1) }
    $wire
}
function Absolute([string]$wire) { $wire -match '^[A-Za-z]:/' }
function Join-Root([string]$relative) { $state.Root + '\' + $relative.Replace('/', '\') }

function Sum-Stream($stream) {
    [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
}

function Sum([string]$path) {
    $file = [IO.File]::OpenRead($path)
    try { Sum-Stream $file } finally { $file.Dispose() }
}

function Is-Link([string]$path) {
    try { ([int][IO.File]::GetAttributes($path) -band [int][IO.FileAttributes]::ReparsePoint) -ne 0 } catch { $false }
}

function Descriptor([string]$path) {
    $sections = [Security.AccessControl.AccessControlSections]'Owner, Access'
    if ([IO.Directory]::Exists($path)) {
        $security = [IO.Directory]::GetAccessControl($path, $sections)
    } else {
        $security = [IO.File]::GetAccessControl($path, $sections)
    }
    New-Object Security.AccessControl.RawSecurityDescriptor -ArgumentList $security.GetSecurityDescriptorBinaryForm(), 0
}

function Owned($descriptor) {
    $owner = $descriptor.Owner
    $null -ne $owner -and ($owner.Value -eq $user -or $owner.Value -eq $tokenOwner)
}

# Whether anyone but this account, the system and the administrators is
# allowed any of the $watched access. A denial takes access away; an entry
# that is neither a plain allowance nor a denial counts as access, since not
# knowing is not the same as nobody having it.
function Others-Can($descriptor, [int]$watched) {
    $acl = $descriptor.DiscretionaryAcl
    if ($null -eq $acl) { return $true }
    foreach ($ace in $acl) {
        if (([int]$ace.AceFlags -band [int][Security.AccessControl.AceFlags]::InheritOnly) -ne 0) { continue }
        $type = [int]$ace.AceType
        if ($type -ne 0) {
            if (@(1, 6, 10, 12) -contains $type) { continue }
            return $true
        }
        if ($trusted -contains $ace.SecurityIdentifier.Value) { continue }
        if (($ace.AccessMask -band $watched) -ne 0) { return $true }
    }
    $false
}

# A folder above the install folder: 0 when no other account can change what
# it holds, 1 when it belongs to another account, 2 when others can change it.
function Safe([string]$path) {
    $descriptor = Descriptor $path
    if (Owned $descriptor) { $watched = $change }
    elseif ($systemOwners -contains $descriptor.Owner.Value) { $watched = $change -band -bnot $addEntry }
    else { return 1 }
    if (Others-Can $descriptor $watched) { return 2 }
    0
}

# A file or folder of this account's that nobody else can change.
function Mine([string]$path) {
    if (Is-Link $path) { return $false }
    $descriptor = Descriptor $path
    (Owned $descriptor) -and -not (Others-Can $descriptor $change)
}

# An access list that gives this account everything and nobody else
# anything, with inheritance cut off, as hide_platform::fs::private makes it;
# a folder's children inherit it.
function Private-Folder {
    $security = New-Object Security.AccessControl.DirectorySecurity
    $security.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$user)")
    $security
}
function Private-File {
    $security = New-Object Security.AccessControl.FileSecurity
    $security.SetSecurityDescriptorSddlForm("D:P(A;;FA;;;$user)")
    $security
}

function Remove-Quietly($path) {
    if ($path) { try { [IO.File]::Delete($path) } catch {} }
}

# $count bytes of the link into a new private file at $path, its missing
# folders made private (the POSIX script's `umask 077; mkdir -p`). Answers
# whether the file holds them all.
function Store([string]$path, [long]$count) {
    $file = $null
    try {
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path), (Private-Folder))
        $file = [IO.File]::Create($path, 65536, [IO.FileOptions]::None, (Private-File))
    } catch { $file = $null }
    try { $whole = Copy-Bytes $count $file } finally { if ($null -ne $file) { $file.Dispose() } }
    $null -ne $file -and $whole
}

# Renames $from to $to, replacing a file there, so $to only ever holds a
# whole copy.
function Replace-Into([string]$from, [string]$to) {
    try {
        if ([IO.File]::Exists($to)) { [IO.File]::Replace($from, $to, [NullString]::Value) }
        else { [IO.File]::Move($from, $to) }
        $true
    } catch { $false }
}

# One word of a Windows command line, as a program's runtime splits it again.
function Quote([string]$word) {
    if ($word.Length -gt 0 -and $word -notmatch '[\s"]') { return $word }
    $quoted = '"'
    $slashes = 0
    foreach ($c in $word.ToCharArray()) {
        if ($c -eq '\') { $slashes += 1; continue }
        if ($c -eq '"') { $quoted += ('\' * (2 * $slashes + 1)) + '"' }
        else { $quoted += ('\' * $slashes) + $c }
        $slashes = 0
    }
    $quoted + ('\' * (2 * $slashes)) + '"'
}
function Command-Line($words) { (@($words) | ForEach-Object { Quote $_ }) -join ' ' }

function Step-Root([string]$wire) {
    if (-not (Absolute $wire)) { Say fail "the install folder $wire is not absolute"; return }
    $native = Native $wire
    try { [void][IO.Directory]::CreateDirectory($native, (Private-Folder)) } catch { Say fail "$wire could not be made"; return }
    try { $full = [IO.Path]::GetFullPath($native).TrimEnd('\') } catch { Say fail "$wire could not be opened"; return }
    $names = $full.Split('\')
    if ($names.Length -lt 2) { Say fail "$wire must be a folder below the drive's root"; return }
    $path = $names[0]
    $bad = $null
    for ($n = 1; $n -lt $names.Length; $n++) {
        $path = $path + '\' + $names[$n]
        if (Is-Link $path) {
            $bad = "$(Wire $path) is a link, so the helper was not installed under it; choose an install root whose folders are real folders"
            break
        }
        if ($n -eq $names.Length - 1) { break }
        try { $safe = Safe $path } catch { $safe = 3 }
        if ($safe -eq 3) { $bad = "$(Wire $path) could not be read, so the helper was not installed under it; choose another install root"; break }
        if ($safe -eq 1) { $bad = "$(Wire $path) belongs to another account, so the helper was not installed under it; choose another install root"; break }
        if ($safe -ne 0) { $bad = "$(Wire $path) can be changed by other accounts, so the helper was not installed under it; remove their write access to it on the device or choose another install root"; break }
    }
    if ($bad) { Say fail $bad; return }
    # The POSIX script's `chmod 700`: a folder of this account's that others
    # can reach is made private, as hide_platform::fs::private::own_dir does.
    $descriptor = Descriptor $full
    if ((Owned $descriptor) -and (Others-Can $descriptor -1)) {
        try { [IO.Directory]::SetAccessControl($full, (Private-Folder)) } catch {}
    }
    if (-not (Mine $full)) { Say fail "$(Wire $full) must be a folder owned by the account and not writable by others; the helper was not installed"; return }
    $state.Root = $full
    Say root (Wire $full)
}

function Step-Held([string]$count) {
    if (-not $state.Root) { Say fail 'no install folder was chosen'; exit 65 }
    for ($n = 0; $n -lt [int]$count; $n++) {
        $pair = (Read-Line).Split([char[]]@([char]9), 2)
        $file = Join-Root $pair[0]
        $held = $false
        try { $held = [IO.File]::Exists($file) -and (Mine $file) -and (Sum $file) -eq $pair[1] } catch { $held = $false }
        if ($held) { Say held $pair[0] }
    }
    Say end
}

function Remove-Stale([string]$folder) {
    try {
        foreach ($file in [IO.Directory]::GetFiles($folder, '.upload-*')) {
            if ([IO.File]::GetLastWriteTimeUtc($file) -lt [DateTime]::UtcNow.AddMinutes(-10)) { Remove-Quietly $file }
        }
    } catch {}
}

function Step-Put([string]$relative, [string]$digest, [string]$size) {
    if (-not $state.Root) { Say fail 'no install folder was chosen'; exit 65 }
    $target = Join-Root $relative
    $folder = [IO.Path]::GetDirectoryName($target)
    $temp = [IO.Path]::Combine($folder, ".upload-$([IO.Path]::GetFileName($target))-$PID")
    Remove-Stale $folder
    $state.Pending = $temp
    if ((Store $temp ([long]$size)) -and (Sum $temp) -eq $digest -and (Replace-Into $temp $target)) {
        $state.Pending = $null
        Say ok
    } else {
        Remove-Quietly $temp
        $state.Pending = $null
        Say fail "$relative did not arrive whole"
    }
}

# Starts $program with $words on the link's own handles, from the account's
# home, after the answer that hands the link over, and ends with it: the
# POSIX script's `exec`, which Windows has no form of.
function Hand-Over([string]$program, $words, [string]$answer) {
    if ($state.At -lt $state.End) { Say fail 'the node sent more than the step'; exit 65 }
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $program
    $info.Arguments = Command-Line $words
    $info.UseShellExecute = $false
    if ($env:USERPROFILE -and [IO.Directory]::Exists($env:USERPROFILE)) { $info.WorkingDirectory = $env:USERPROFILE }
    Say $answer
    $child = [Diagnostics.Process]::Start($info)
    $child.WaitForExit()
    exit $child.ExitCode
}

function Step-Start([string]$relative, [string]$herdr, [string]$homeWire) {
    if (-not $state.Root) { Say fail 'no install folder was chosen'; exit 65 }
    if ($herdr -ne '-') { Say fail 'a Windows device runs no Herdr, so it takes no Herdr session or socket'; exit 65 }
    $helper = Join-Root $relative
    if (-not [IO.File]::Exists($helper)) { Say fail "the helper $(Wire $helper) is missing"; exit 65 }
    # A home of the node's own in place of the account's, made private where
    # it is missing: the account's home is what Windows programs read it from.
    if ($homeWire -and $homeWire -ne '-') {
        try {
            if (-not (Absolute $homeWire)) { throw 'not absolute' }
            [void][IO.Directory]::CreateDirectory((Native $homeWire), (Private-Folder))
            $env:USERPROFILE = Native $homeWire
        } catch { Say fail "the home $homeWire could not be made"; exit 65 }
    }
    Hand-Over $helper @('node', 'serve', '--own-herdr') 'start'
}

function Step-Attach([string]$program, [string]$stateDir) {
    if (-not (Absolute $program) -or -not [IO.File]::Exists((Native $program))) { Say fail "$program is missing"; exit 65 }
    if ($stateDir -eq '-') { $words = @('attach') } else { $words = @('attach', '--state-dir', (Native $stateDir)) }
    Hand-Over (Native $program) $words 'attach'
}

# Runs the command with no input and answers its status and both outputs,
# each kept to $cap bytes: one that writes past it is ended with what it
# started, and refused.
function Step-Run([string]$count, [string]$cap) {
    $words = New-Object Collections.Generic.List[string]
    for ($n = 0; $n -lt [int]$count; $n++) { $words.Add((Read-Line)) }
    $limit = [long]$cap
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $words[0]
    $info.Arguments = Command-Line $words.GetRange(1, $words.Count - 1)
    $info.UseShellExecute = $false
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    try { $child = [Diagnostics.Process]::Start($info) } catch {
        $said = $utf8.GetBytes("$($words[0]): the command could not be started`n")
        Say ran 127 0 $said.Length
        $stdout.Write($said, 0, $said.Length)
        $stdout.Flush()
        return
    }
    $child.StandardInput.Close()
    $streams = @($child.StandardOutput.BaseStream, $child.StandardError.BaseStream)
    $kept = @((New-Object IO.MemoryStream), (New-Object IO.MemoryStream))
    $chunks = @((New-Object byte[] 65536), (New-Object byte[] 65536))
    $reads = @($streams[0].ReadAsync($chunks[0], 0, 65536), $streams[1].ReadAsync($chunks[1], 0, 65536))
    $big = $false
    while (-not $big -and ($reads[0] -or $reads[1])) {
        [void][Threading.Tasks.Task]::WaitAny([Threading.Tasks.Task[]]@($reads | Where-Object { $_ }))
        for ($k = 0; $k -lt 2; $k++) {
            $read = $reads[$k]
            if (-not $read -or -not $read.IsCompleted) { continue }
            $n = $read.Result
            if ($n -le 0) { $reads[$k] = $null; continue }
            $kept[$k].Write($chunks[$k], 0, $n)
            if ($kept[$k].Length -gt $limit) { $big = $true; continue }
            $reads[$k] = $streams[$k].ReadAsync($chunks[$k], 0, 65536)
        }
    }
    if ($big) {
        # Its whole tree, so nothing it started keeps writing into a pipe
        # nobody reads.
        try {
            $kill = New-Object Diagnostics.ProcessStartInfo "$env:SystemRoot\System32\taskkill.exe", "/T /F /PID $($child.Id)"
            $kill.UseShellExecute = $false
            $kill.RedirectStandardInput = $true
            $kill.RedirectStandardOutput = $true
            $kill.RedirectStandardError = $true
            $killer = [Diagnostics.Process]::Start($kill)
            $killer.StandardInput.Close()
            [void]$killer.StandardOutput.ReadToEnd()
            [void]$killer.StandardError.ReadToEnd()
            $killer.WaitForExit()
        } catch {}
    }
    $child.WaitForExit()
    $status = $child.ExitCode
    if ($big) {
        Say big $status ([Math]::Min($kept[0].Length, $limit + 1)) ([Math]::Min($kept[1].Length, $limit + 1))
        return
    }
    Say ran $status $kept[0].Length $kept[1].Length
    $stdout.Write($kept[0].GetBuffer(), 0, [int]$kept[0].Length)
    $stdout.Write($kept[1].GetBuffer(), 0, [int]$kept[1].Length)
    $stdout.Flush()
}

function Step-Fput([string]$wire, [string]$digest, [string]$size) {
    if (-not (Absolute $wire)) { [void](Copy-Bytes ([long]$size) $null); Say fail "$wire is not absolute"; return }
    $target = Native $wire
    $temp = "$target.part"
    $state.Pending = $temp
    if ((Store $temp ([long]$size)) -and (Sum $temp) -eq $digest -and (Replace-Into $temp $target)) {
        $state.Pending = $null
        Say ok
    } else {
        Remove-Quietly $temp
        $state.Pending = $null
        Say fail "$wire did not arrive whole"
    }
}

function Step-Fget([string]$wire) {
    $file = $null
    if (Absolute $wire) {
        try { $file = [IO.File]::Open((Native $wire), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read) } catch { $file = $null }
    }
    if ($null -eq $file) { Say fail "$wire is not a file Hide can read"; return }
    try {
        $size = $file.Length
        $digest = Sum-Stream $file
        [void]$file.Seek(0, [IO.SeekOrigin]::Begin)
        Say file $size $digest
        $chunk = New-Object byte[] 65536
        $left = $size
        while ($left -gt 0) {
            $n = $file.Read($chunk, 0, [int][Math]::Min([long]$chunk.Length, $left))
            if ($n -le 0) { throw 'the file got shorter while it was read' }
            $stdout.Write($chunk, 0, $n)
            $left -= $n
        }
        $stdout.Flush()
    } finally { $file.Dispose() }
}

function Probe {
    $machine = $env:HIDE_MACHINE_ID
    if (-not $machine) {
        try {
            $key = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
            $machine = [string]$key.OpenSubKey('SOFTWARE\Microsoft\Cryptography').GetValue('MachineGuid')
        } catch { $machine = $null }
    }
    if (-not $machine) { $machine = '-' }
    $arch = $env:PROCESSOR_ARCHITEW6432
    if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
    $arch = ([string]$arch).ToLowerInvariant()
    $homeFolder = [string]$env:USERPROFILE
    if ($homeFolder) { $homeFolder = Wire $homeFolder }
    Say probe ([Environment]::UserName) $homeFolder Windows $arch $machine yes
}

try {
    Probe
    while ($true) {
        $fields = (Read-Line).Split([char[]]@([char]9), 5)
        switch -CaseSensitive ($fields[0]) {
            'root' { Step-Root $fields[1] }
            'held' { Step-Held $fields[1] }
            'put' { Step-Put $fields[1] $fields[2] $fields[4] }
            'start' { Step-Start $fields[1] $fields[2] $fields[3] }
            'run' { Step-Run $fields[1] $fields[2] }
            'fput' { Step-Fput $fields[1] $fields[2] $fields[3] }
            'fget' { Step-Fget $fields[1] }
            'attach' { Step-Attach $fields[1] $fields[2] }
            default { Say fail "unknown step $($fields[0])"; exit 64 }
        }
    }
} catch {
    Remove-Quietly $state.Pending
    $failure = $_.Exception
    while ($failure -is [Management.Automation.MethodInvocationException] -and $failure.InnerException) { $failure = $failure.InnerException }
    if ($failure -is [IO.EndOfStreamException]) { exit 0 }
    [Console]::Error.WriteLine("Hide's start script failed: $($failure.Message)")
    exit 65
}
