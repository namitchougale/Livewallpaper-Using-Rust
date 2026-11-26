param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Mp4
)
$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path -LiteralPath $Exe).Path
$videoPath = (Resolve-Path -LiteralPath $Mp4).Path
if (-not ('FlowMeasure.Window' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace FlowMeasure {
    public static class Window {
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string name, string title);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
    }
}
'@
}
$class = 'Ypar.Flow.MP4.Panel.11'
if ([FlowMeasure.Window]::FindWindow($class, $null) -ne [IntPtr]::Zero) {
    throw 'Quit the open copy of Flow before measuring a separate session.'
}
function Measure-Interval($Process, $State, $Seconds) {
    $Process.Refresh()
    $baseline = $Process.TotalProcessorTime
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $peakPrivate = 0L
    while ($watch.Elapsed.TotalSeconds -lt $Seconds) {
        Start-Sleep -Milliseconds 200
        $Process.Refresh()
        if ($Process.HasExited) { throw "Flow exited while $State." }
        $peakPrivate = [Math]::Max([long]$peakPrivate, $Process.PrivateMemorySize64)
        if ($peakPrivate -gt 140000000 -or $Process.PeakWorkingSet64 -gt 140000000) {
            throw "The 140 MB process limit was exceeded while $State."
        }
    }
    $cpu = ($Process.TotalProcessorTime - $baseline).TotalSeconds / $watch.Elapsed.TotalSeconds
    [pscustomobject]@{
        State = $State
        Seconds = [Math]::Round($watch.Elapsed.TotalSeconds, 2)
        CPUPercentOfMachine = [Math]::Round(100 * $cpu / [Environment]::ProcessorCount, 2)
        CpuMsPerSecond = [Math]::Round(1000 * $cpu, 1)
        PeakPrivateMB = [Math]::Round($peakPrivate / 1000000, 1)
        LifetimePeakWorkingSetMB = [Math]::Round($Process.PeakWorkingSet64 / 1000000, 1)
    }
}
$sandbox = Join-Path ([IO.Path]::GetTempPath()) ('Flow-Measure-' + [guid]::NewGuid())
$profileDir = Join-Path $sandbox 'FlowLiveWallpaper'
$null = [IO.Directory]::CreateDirectory($profileDir)
$process = $null
$panel = [IntPtr]::Zero
try {
    [IO.File]::WriteAllText((Join-Path $profileDir 'video.txt'), $videoPath)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $exePath
    $start.Arguments = '--preview'
    $start.UseShellExecute = $false
    $start.EnvironmentVariables['LOCALAPPDATA'] = $sandbox
    $process = [Diagnostics.Process]::Start($start)
    $saved = Join-Path $profileDir 'library-v1.txt'
    $timeout = [Diagnostics.Stopwatch]::StartNew()
    do {
        if ($process.HasExited) { throw 'Flow exited before opening the test MP4.' }
        $panel = [FlowMeasure.Window]::FindWindow($class, $null)
        [uint32]$owner = 0
        if ($panel -ne [IntPtr]::Zero) {
            $null = [FlowMeasure.Window]::GetWindowThreadProcessId($panel, [ref]$owner)
            if ($owner -ne $process.Id) { throw 'Another Flow session appeared during the test.' }
        }
        $ready = (Test-Path -LiteralPath $saved) -and (([IO.File]::ReadAllLines($saved))[1] -eq '3')
        if ($panel -ne [IntPtr]::Zero -and $ready) { break }
        if ($timeout.Elapsed.TotalSeconds -gt 30) { throw 'Flow did not confirm the first MP4 frame within 30 seconds.' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    Measure-Interval $process 'Playing' 8
    $result = [IntPtr]::Zero
    if ([FlowMeasure.Window]::SendMessageTimeout($panel, 0x111, [IntPtr]108, [IntPtr]::Zero, 2, 3000, [ref]$result) -eq [IntPtr]::Zero) {
        throw 'The panel did not respond to Pause.'
    }
    Measure-Interval $process 'Paused' 5
    if (-not [FlowMeasure.Window]::PostMessage($panel, 0x111, [IntPtr]110, [IntPtr]::Zero) -or -not $process.WaitForExit(5000)) {
        throw 'The panel did not quit cleanly.'
    }
    if ($process.ExitCode -ne 0) { throw "Flow exited with code $($process.ExitCode)." }
} finally {
    if ($null -ne $process) {
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
    Remove-Item -LiteralPath $sandbox -Recurse -Force
}
