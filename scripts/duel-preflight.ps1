[CmdletBinding()]
param(
    [ValidateRange(0.0, 64.0)]
    [double]$MinimumFreeGiB = 2.0
)

$memory = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
$freeGiB = $memory.FreePhysicalMemory / 1MB

if ($freeGiB -lt $MinimumFreeGiB) {
    Write-Error (
        "Duel preflight rejected: {0:N2} GiB free; at least {1:N2} GiB is required." -f
        $freeGiB,
        $MinimumFreeGiB
    )
    exit 3
}

Write-Output (
    "Duel preflight passed: {0:N2} GiB free (minimum {1:N2} GiB)." -f
    $freeGiB,
    $MinimumFreeGiB
)
