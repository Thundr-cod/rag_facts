# pick-model.ps1
# Looks at every .gguf in .system, keeps the ones that fit this PC's memory, benchmarks a few
# (biggest first, GPU vs CPU via bench-decide.ps1) and prints ONE line for WindowsLaunch.bat:
#   OK|<model path>|<GPU or CPU>|<tokens/sec>|<other device tokens/sec>
#   FAIL|<reason to show the user>
# The pick is cached per computer and per set of model files in .system\.model-choice-<PC>.txt,
# so moving the drive to another PC (or adding/removing a model) re-runs the test.

param(
    [string]$SystemDir,
    [int]$CtxSize = 8192,
    [double]$MinTps = 5,        # below this, chat feels broken (slower than reading speed)
    [int]$MaxCandidates = 3     # how many models to benchmark at most (~30-90 sec each)
)

$ErrorActionPreference = "SilentlyContinue"

# GB a model needs: weights + KV cache for the context + ~0.5 GB runtime.
# shortcut: KV is scaled from Qwen3.5-4B (~0.14 GB per 1K context per 4.5 GB of weights); real KV
# size depends on the architecture, so the benchmark is the real judge. Upgrade: read GGUF metadata.
function Get-NeedGB([double]$FileGB, [int]$Ctx) {
    return $FileGB + ($Ctx / 1024) * 0.14 * ($FileGB / 4.5) + 0.5
}

# Models that fit, biggest first (bigger file ~ better answers). If more fit than we will bench,
# keep the biggest few plus the smallest, so a FAIL means even the smallest one is too slow.
function Select-Candidates($Models, [double]$BudgetGB, [int]$Ctx, [int]$Max) {
    $fit = @($Models | Where-Object { (Get-NeedGB $_.GB $Ctx) -le $BudgetGB } | Sort-Object GB -Descending)
    if ($fit.Count -le $Max) { return $fit }
    return @($fit[0..($Max - 2)]) + $fit[-1]
}

if ($MyInvocation.InvocationName -eq '.') { return }   # dot-sourced by the self-test: functions only

# shortcut: split models (name-00001-of-00003.gguf) are sized by each part alone. Upgrade if anyone ships them.
$all = @(Get-ChildItem -Path $SystemDir -Filter *.gguf -File | Where-Object { $_.Name -notmatch 'mmproj' } |
    ForEach-Object { [pscustomobject]@{ Name = $_.Name; Path = $_.FullName; GB = $_.Length / 1GB } })
if ($all.Count -eq 0) { Write-Output "FAIL|No .gguf model files found in the .system folder."; exit }

$cacheFile = Join-Path $SystemDir ".model-choice-$env:COMPUTERNAME.txt"
$sig = (($all | Sort-Object Name | ForEach-Object { "$($_.Name):$([long]($_.GB * 1GB))" }) -join ';') + "|ctx=$CtxSize"
$cache = @(Get-Content $cacheFile)
if ($cache.Count -ge 2 -and $cache[0] -eq $sig -and (Test-Path ($cache[1] -split '\|')[1])) {
    Write-Output $cache[1]; exit
}

# Usable memory: RAM minus ~2 GB for Windows, plus NVIDIA VRAM (-ngl auto spills what doesn't fit).
# shortcut: AMD/Intel VRAM isn't read (Windows caps it at 4 GB), so those count as RAM-only.
$ramGB = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB
$vramGB = 0
$smi = & nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits 2>$null
if ($smi) { $vramGB = [double]("$($smi | Select-Object -First 1)".Trim()) / 1024 }
$budgetGB = ($ramGB - 2) + $vramGB

$cands = @(Select-Candidates $all $budgetGB $CtxSize $MaxCandidates)
if ($cands.Count -eq 0) {
    $small = $all | Sort-Object GB | Select-Object -First 1
    Write-Output ("FAIL|Every model is too big for this PC. The smallest ({0}) needs about {1:N1} GB, this PC has about {2:N1} GB usable. Add a smaller model (for example a Q4_K_M of a 1-4B model)." -f $small.Name, (Get-NeedGB $small.GB $CtxSize), $budgetGB)
    exit
}

$bench = Join-Path $PSScriptRoot 'llama-bench.exe'
$decide = Join-Path $PSScriptRoot 'bench-decide.ps1'
$fastest = $null
foreach ($m in $cands) {
    [Console]::Error.WriteLine("  Testing $($m.Name) ({0:N1} GB)..." -f $m.GB)
    $winner, $tps, $other = "$(& $decide $bench $m.Path)" -split '\|'
    if ($winner -ne 'GPU' -and $winner -ne 'CPU') { continue }
    $line = "OK|$($m.Path)|$winner|$tps|$other"
    if ([double]$tps -ge $MinTps) {
        # Only cache when both devices were measured; a one-sided result may be a transient failure.
        if ([double]$other -gt 0) { Set-Content -Path $cacheFile -Value @($sig, $line) }
        Write-Output $line; exit
    }
    if (-not $fastest -or [double]$tps -gt [double]$fastest.Tps) { $fastest = @{ Name = $m.Name; Tps = $tps } }
}

if (-not $fastest) {
    # Every benchmark broke (often antivirus holding llama-bench on first run): don't lock anything in,
    # run the smallest model that fits on CPU this session and test again next launch.
    $m = $cands | Sort-Object GB | Select-Object -First 1
    Write-Output "OK|$($m.Path)|CPU|0|0"; exit
}

Write-Output ("FAIL|Every model that fits runs too slowly on this PC (best: {0} at {1} tokens/sec, need {2}+). Add a smaller model (for example a Q4_K_M of a 1-4B model)." -f $fastest.Name, $fastest.Tps, $MinTps)
