param(
    [string]$Executable = (Join-Path $PSScriptRoot "..\target\release\nxnote.exe"),
    [int]$SampleSeconds = 15,
    [double]$MaxOneCorePercent = 50
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Windows.Forms
$process = Start-Process -FilePath $Executable -PassThru
try {
    for ($i = 0; $i -lt 30 -and $process.MainWindowHandle -eq 0; $i++) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
    }
    if ($process.MainWindowHandle -eq 0) {
        throw 'NxNote main window was not created.'
    }

    $shell = New-Object -ComObject WScript.Shell
    if (-not $shell.AppActivate($process.Id)) {
        throw 'Could not activate the NxNote window.'
    }
    Start-Sleep -Milliseconds 500
    # Tab focuses a UI control without changing the current note.
    [System.Windows.Forms.SendKeys]::SendWait('{TAB}')
    Start-Sleep -Seconds 1

    $before = Get-Process -Id $process.Id
    $start = Get-Date
    Start-Sleep -Seconds $SampleSeconds
    $after = Get-Process -Id $process.Id

    $elapsed = ((Get-Date) - $start).TotalSeconds
    $oneCorePercent = (($after.CPU - $before.CPU) / $elapsed) * 100
    [pscustomobject]@{
        SampleSeconds = [math]::Round($elapsed, 1)
        OneCorePercent = [math]::Round($oneCorePercent, 1)
        PrivateMiB = [math]::Round($after.PrivateMemorySize64 / 1MB, 1)
        Handles = $after.Handles
    } | Format-List

    if ($oneCorePercent -gt $MaxOneCorePercent) {
        throw "Focused CPU use was $([math]::Round($oneCorePercent, 1))%, above the $MaxOneCorePercent% limit."
    }
}
finally {
    if (Get-Process -Id $process.Id -ErrorAction SilentlyContinue) {
        Stop-Process -Id $process.Id
    }
}
