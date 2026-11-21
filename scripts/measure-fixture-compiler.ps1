# Read the compiler images sequentially before starting them. Together with
# fixture.compile.work this separates slow image reads from actual C work;
# it does not attribute a slow read to Defender or to a particular disk.
$ErrorActionPreference = 'Stop'
$chunk = [byte[]]::new(4MB)
foreach ($name in @('clang.exe', 'lld-link.exe')) {
    $program = (Get-Command $name -CommandType Application).Source
    foreach ($pass in @('cold', 'warm')) {
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $stream = [IO.File]::OpenRead($program)
        try {
            $bytes = 0L
            while (($read = $stream.Read($chunk, 0, $chunk.Length)) -gt 0) { $bytes += $read }
        } finally {
            $stream.Dispose()
        }
        $clock.Stop()
        @{ kind = 'fixture.compiler.read'; program = $name; pass = $pass; bytes = $bytes; elapsed_ms = $clock.ElapsedMilliseconds } |
            ConvertTo-Json -Compress
    }
}
