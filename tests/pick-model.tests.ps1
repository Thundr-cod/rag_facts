# Self-check for pick-model.ps1's sizing/selection logic. Run: powershell -File tests\pick-model.tests.ps1
$ErrorActionPreference = "Stop"
. "$PSScriptRoot\..\.system\windows\pick-model.ps1"

function M($name, $gb) { [pscustomobject]@{ Name = $name; Path = "x\$name"; GB = $gb } }
$models = @((M 'tiny' 1), (M 'small' 2.5), (M 'mid' 4.5), (M 'big' 8), (M 'huge' 40))
$fail = 0
function Check($label, $got, $want) {
    if ("$got" -ne "$want") { Write-Host "FAIL $label : got '$got' want '$want'"; $script:fail++ } else { Write-Host "ok   $label" }
}

# Stock 4.5 GB model at 32K context: ~4.5 + 4.5 KV + 0.5 = ~9.5 GB (measured ~9.9 GB on an RTX 5070).
Check 'need 4.5GB @32K' ([math]::Round((Get-NeedGB 4.5 32768), 1)) 9.5
# 16 GB budget @8K: huge drops out; 4 fit, so bench the 2 biggest plus the smallest.
Check 'fit 16GB' ((Select-Candidates $models 16 8192 3).Name -join ',') 'big,mid,tiny'
# Only as many fit as we bench: all of them, biggest first.
Check 'fit 7GB' ((Select-Candidates $models 7 8192 3).Name -join ',') 'mid,small,tiny'
# More fit than we bench: keep biggest (Max-1) plus the smallest.
Check 'fit 16GB max2' ((Select-Candidates $models 16 8192 2).Name -join ',') 'big,tiny'
# Budget below every model: nothing to bench, so the picker fails out.
Check 'nothing fits' (@(Select-Candidates $models 1 8192 3).Count) 0

if ($fail) { exit 1 }
