<#
.SYNOPSIS
  The model ladder: run `crab-cage run` with a local model on nuggets of
  rising difficulty, several times each, and write one table of results.

.DESCRIPTION
  For every model in the list and every nugget, this script runs the search
  loop -Reps times with the local-model proposer and records, per run:
  whether the nugget was solved, the best proven score, how many candidates
  were journaled, the wall time, where candidates were rejected, and which
  lust error codes came up.

  It needs: this repository (the script builds it), `rustc` on PATH, and an Ollama server on this machine. A lust
  binary is optional; without one `rustc` does the fast gates and no lust
  error codes are collected.

  Works in Windows PowerShell 5.1 and in PowerShell 7 (also on Linux/macOS).

.EXAMPLE
  .\experiments\model-ladder\run.ps1 -Pull -LustBin C:\lust\bin\lust.exe

.EXAMPLE
  .\experiments\model-ladder\run.ps1 -Models qwen2.5-coder:14b -Nuggets normalize-path -Reps 5
#>
[CmdletBinding()]
param(
    # Model names as Ollama knows them. Default: the lines of models.txt.
    [string[]]$Models,
    # Nugget directory names under nuggets/. Default: the four examples, easy to hard.
    [string[]]$Nuggets = @('sum-squares', 'run-lengths', 'merge-ranges', 'normalize-path'),
    # Runs per model and nugget. A model's replies vary, so one run says little.
    [int]$Reps = 3,
    # Candidates to journal per run, the seed included.
    [int]$Budget = 20,
    # A lust binary. Empty: rustc does the fast gates.
    [string]$LustBin = '',
    # The chat endpoint. Must be loopback; crab-cage refuses anything else.
    [string]$Endpoint = 'http://127.0.0.1:11434/v1',
    # Where results go. Default: experiments/model-ladder/results/<date-time>.
    [string]$OutDir = '',
    # Download a model that is not installed (`ollama pull`). Without this
    # switch a missing model is skipped and named in the summary.
    [switch]$Pull,
    # Also run the literal-tweak proposer, taking turns with the model. Worth
    # it on a `minimize` nugget whose candidates carry tunable numbers.
    [switch]$Tweak,
    # One model's first nugget, one run, budget 6: a two-minute check that
    # everything is wired up.
    [switch]$Quick
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
$utf8 = New-Object System.Text.UTF8Encoding($false)

function Write-Text([string]$Path, [string]$Text) {
    [System.IO.File]::WriteAllText($Path, $Text, $utf8)
}

function Get-Median([double[]]$Values) {
    if (-not $Values -or $Values.Count -eq 0) { return $null }
    $sorted = @($Values | Sort-Object)
    $mid = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$mid] }
    return ($sorted[$mid - 1] + $sorted[$mid]) / 2
}

function Invoke-Tool([string]$Exe, [string[]]$Arguments) {
    # First line of a tool's output, or '' when the tool is missing or fails.
    try {
        $out = & $Exe @Arguments 2>$null
        if ($out) { return [string](@($out)[0]) }
    } catch { }
    return ''
}

# ---------------------------------------------------------------- inputs
if (-not $Models -or $Models.Count -eq 0) {
    $Models = @(Get-Content (Join-Path $here 'models.txt') |
        ForEach-Object { $_.Trim() } |
        Where-Object { $_ -and -not $_.StartsWith('#') })
}
# `powershell -File run.ps1 -Nuggets a,b` passes one string, 'a,b'. Split it.
$Models = @($Models | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$Nuggets = @($Nuggets | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($Quick) {
    $Models = @($Models[0])
    $Nuggets = @($Nuggets[0])
    $Reps = 1
    $Budget = 6
}
if (-not $OutDir) {
    $stamp = Get-Date -Format 'yyyy-MM-dd_HHmm'
    $OutDir = Join-Path $here (Join-Path 'results' $stamp)
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

foreach ($nugget in $Nuggets) {
    if (-not (Test-Path (Join-Path $repo (Join-Path 'nuggets' $nugget)))) {
        throw "no nugget named '$nugget' under $repo\nuggets"
    }
}
if ($LustBin -and -not (Test-Path $LustBin)) { throw "lust binary not found: $LustBin" }
if ($LustBin) { $LustBin = (Resolve-Path $LustBin).Path }

$objective = @{}
$promptChars = 0
foreach ($nugget in $Nuggets) {
    $dir = Join-Path $repo (Join-Path 'nuggets' $nugget)
    $objective[$nugget] = (Get-Content (Join-Path $dir 'nugget.json') -Raw | ConvertFrom-Json).objective
    $size = (Get-Item (Join-Path $dir 'template.rs')).Length
    if ($size -gt $promptChars) { $promptChars = $size }
}
# A rough upper bound for one prompt, in tokens: the template, lust's
# capability summary (about 4,000 characters), the region again, feedback and
# the "already tried" list (about 6,000), at about 3 characters per token.
$promptTokens = [int](($promptChars + 4000 + 6000) / 3)

# ---------------------------------------------------------------- crab-cage
$exeName = 'crab-cage'
if ($env:OS -eq 'Windows_NT') { $exeName = 'crab-cage.exe' }
$crab = Join-Path $repo (Join-Path 'target' (Join-Path 'release' $exeName))
# Always build: it is quick when nothing changed, and an old binary would
# silently run an old prompt.
Write-Host 'building crab-cage (cargo build --release) ...'
Push-Location $repo
try { & cargo build --release --locked; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } }
finally { Pop-Location }
if (-not (Test-Path $crab)) { throw "the build did not produce $crab" }

# ---------------------------------------------------------------- the model server
$base = $Endpoint -replace '/v1/?$', ''
try { $tags = Invoke-RestMethod -Uri "$base/api/tags" -TimeoutSec 10 }
catch { throw "no Ollama server answered at $base. Start Ollama, then run this again. ($($_.Exception.Message))" }
$installed = @($tags.models | ForEach-Object { $_.name })
$ollamaVersion = ''
try { $ollamaVersion = (Invoke-RestMethod -Uri "$base/api/version" -TimeoutSec 10).version } catch { }

function Test-Installed([string]$Model) {
    # `qwen2.5-coder:7b` matches itself; a bare `name` matches `name:latest`.
    foreach ($name in $script:installed) {
        if ($name -eq $Model -or $name -eq "${Model}:latest") { return $true }
    }
    return $false
}

$skipped = @()
$ready = @()
foreach ($model in $Models) {
    if (Test-Installed $model) { $ready += $model; continue }
    if ($Pull) {
        Write-Host "pulling $model ..."
        & ollama pull $model
        if ($LASTEXITCODE -eq 0) { $ready += $model; continue }
        $skipped += "$model (ollama pull failed)"
    } else {
        $skipped += "$model (not installed; run with -Pull, or: ollama pull $model)"
    }
}
if ($ready.Count -eq 0) { throw "none of the models is installed: $($Models -join ', '). Run with -Pull." }

# ---------------------------------------------------------------- the machine
$cpu = ''; $gpus = @(); $ramGb = $null
try {
    if ($env:OS -eq 'Windows_NT') {
        $cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
        $gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name })
        $ramGb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
    }
} catch { }
$nvidia = Invoke-Tool 'nvidia-smi' @('--query-gpu=name,memory.total', '--format=csv,noheader')
$environment = [ordered]@{
    date           = (Get-Date -Format 'yyyy-MM-dd HH:mm')
    cpu            = $cpu
    gpus           = $gpus
    nvidia_smi     = $nvidia
    ram_gb         = $ramGb
    os             = [System.Environment]::OSVersion.VersionString
    ollama         = $ollamaVersion
    crab_cage      = (Invoke-Tool 'git' @('-C', $repo, 'rev-parse', '--short', 'HEAD'))
    rustc          = (Invoke-Tool 'rustc' @('--version'))
    lust           = $(if ($LustBin) { Invoke-Tool $LustBin @('--version') } else { '' })
    endpoint       = $Endpoint
    reps           = $Reps
    budget         = $Budget
    tweak          = [bool]$Tweak
    prompt_tokens  = $promptTokens
    nuggets        = $Nuggets
    models         = $ready
    skipped_models = $skipped
}
Write-Text (Join-Path $OutDir 'environment.json') ($environment | ConvertTo-Json -Depth 4)

# ---------------------------------------------------------------- one run
function Read-Journal([string]$Path) {
    $rows = @()
    if (Test-Path $Path) {
        foreach ($line in [System.IO.File]::ReadAllLines($Path)) {
            if ($line.Trim()) { try { $rows += ($line | ConvertFrom-Json) } catch { } }
        }
    }
    return , $rows
}

function Invoke-Run([string]$Model, [string]$Nugget, [int]$Rep) {
    $safe = $Model -replace '[^A-Za-z0-9._-]', '_'
    $out = Join-Path $OutDir (Join-Path $safe (Join-Path $Nugget "rep-$Rep"))
    if (Test-Path $out) { Remove-Item -Recurse -Force $out }
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    $arguments = @('run', (Join-Path $repo (Join-Path 'nuggets' $Nugget)),
        '--backend', 'local', '--endpoint', $Endpoint, '--model', $Model,
        '--budget', "$Budget", '--jobs', '1', '--out', $out)
    if ($LustBin) { $arguments += @('--lust-bin', $LustBin) }
    if ($Tweak) { $arguments += '--tweak' }

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $log = & $crab @arguments 2>&1 | ForEach-Object { "$_" }
    $exit = $LASTEXITCODE
    $ErrorActionPreference = $previous
    $watch.Stop()
    Write-Text (Join-Path $out 'run.log') (($log -join "`n") + "`n")

    $rows = Read-Journal (Join-Path $out 'journal.jsonl')
    $model_rows = @($rows | Where-Object { $_.proposer -ne 'seed' })
    $proven = @($rows | Where-Object { $_.status -eq 'proven' })
    $best = $null
    foreach ($row in $proven) { if ($null -eq $best -or $row.score -lt $best) { $best = $row.score } }
    $seed = $rows | Where-Object { $_.proposer -eq 'seed' } | Select-Object -First 1
    $solvedAt = $null
    foreach ($row in $rows) {
        if ($row.status -eq 'proven' -and $row.score -eq 0 -and $row.proposer -ne 'seed') { $solvedAt = $row.n - 1; break }
    }
    $gates = [ordered]@{ prefilter = 0; check = 0; build = 0; run = 0; oracle = 0 }
    $codes = @{}
    $toolMs = 0
    foreach ($row in $rows) {
        $toolMs += ($row.ms.check + $row.ms.build + $row.ms.run + $row.ms.oracle)
    }
    foreach ($row in $model_rows) {
        if ($row.status -eq 'rejected') { $gates[$row.gate] = $gates[$row.gate] + 1 }
        foreach ($diagnostic in @($row.diagnostics)) {
            if ($diagnostic.code) { $codes[$diagnostic.code] = 1 + [int]$codes[$diagnostic.code] }
        }
    }
    $stop = @($log | Where-Object { $_ -match '^(stopped:|local proposer finished:)' }) | Select-Object -First 1
    return [ordered]@{
        model               = $Model
        nugget              = $Nugget
        rep                 = $Rep
        exit                = $exit
        objective           = $objective[$Nugget]
        # pass_all: a proven candidate with score 0. minimize has no "solved";
        # `improved` says whether a proven candidate beat the seed.
        solved              = ($objective[$Nugget] -eq 'pass_all' -and $exit -eq 0)
        improved            = ($null -ne $best -and $seed -and $best -lt $seed.score)
        seed_score          = $(if ($seed) { $seed.score } else { $null })
        best_score          = $best
        candidates          = $model_rows.Count
        solved_at_candidate = $solvedAt
        diverged            = @($rows | Where-Object { $_.status -eq 'diverged' }).Count
        rejected            = $gates
        codes               = $codes
        wall_s              = [math]::Round($watch.Elapsed.TotalSeconds, 1)
        tools_s             = [math]::Round($toolMs / 1000, 1)
        stop                = $(if ($stop) { "$stop" } else { '' })
        # Relative to the results directory, so a shared result names no home directory.
        out                 = (Join-Path $safe (Join-Path $Nugget "rep-$Rep"))
    }
}

# ---------------------------------------------------------------- the ladder
$results = @()
$loaded = @()
$total = $ready.Count * $Nuggets.Count * $Reps
$done = 0
foreach ($model in $ready) {
    Write-Host ''
    Write-Host "== $model"
    # Load the model before the first timed run, and see where it sits.
    $body = @{ model = $model; messages = @(@{ role = 'user'; content = 'Reply with the word ready.' }) } | ConvertTo-Json -Depth 5
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    try { $null = Invoke-RestMethod -Uri "$Endpoint/chat/completions" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 900 }
    catch { Write-Host "   could not load ${model}: $($_.Exception.Message)"; $skipped += "$model (did not answer a first request)"; continue }
    $watch.Stop()
    $placement = [ordered]@{ model = $model; load_s = [math]::Round($watch.Elapsed.TotalSeconds, 1); size_gb = $null; vram_gb = $null; context = $null; context_ok = $true }
    try {
        $entry = (Invoke-RestMethod -Uri "$base/api/ps" -TimeoutSec 10).models | Where-Object { $_.name -eq $model -or $_.name -eq "${model}:latest" } | Select-Object -First 1
        if ($entry) {
            $placement.size_gb = [math]::Round($entry.size / 1e9, 1)
            $placement.vram_gb = [math]::Round($entry.size_vram / 1e9, 1)
            $placement.context = $entry.context_length
        }
    } catch { }
    if ($placement.context -and $placement.context -lt $promptTokens) { $placement.context_ok = $false }
    $loaded += $placement
    Write-Host "   loaded in $($placement.load_s) s; $($placement.vram_gb) of $($placement.size_gb) GB on the GPU; context $($placement.context) tokens"
    if (-not $placement.context_ok) {
        Write-Warning "the largest prompt here is about $promptTokens tokens and this model's context is $($placement.context). Ollama cuts a prompt that does not fit, silently. Stop Ollama, set OLLAMA_CONTEXT_LENGTH (16384 or 32768), start it again, and re-run."
    }

    foreach ($nugget in $Nuggets) {
        for ($rep = 1; $rep -le $Reps; $rep++) {
            $done++
            $result = Invoke-Run $model $nugget $rep
            $results += $result
            $mark = 'not solved'
            if ($result.solved) { $mark = "solved at candidate $($result.solved_at_candidate)" }
            if ($result.objective -ne 'pass_all') { $mark = "seed $($result.seed_score)" }
            Write-Host ("   [{0}/{1}] {2} rep {3}: {4}, best {5}, {6} candidates, {7} s" -f `
                    $done, $total, $nugget, $rep, $mark, $result.best_score, $result.candidates, $result.wall_s)
            [System.IO.File]::AppendAllText((Join-Path $OutDir 'results.jsonl'), (($result | ConvertTo-Json -Depth 5 -Compress) + "`n"), $utf8)
        }
    }
}

# ---------------------------------------------------------------- the summary
$lines = @()
$lines += '# Model ladder'
$lines += ''
$lines += "- Date: $($environment.date)"
$lines += "- CPU: $cpu"
$lines += "- GPU: $((@($gpus) -join '; '))$(if ($nvidia) { " (nvidia-smi: $nvidia)" })"
$lines += "- RAM: $ramGb GB"
$lines += "- Ollama: $ollamaVersion"
$lines += "- crab-cage: $($environment.crab_cage); $($environment.rustc)"
$lines += "- lust: $(if ($environment.lust) { $environment.lust } else { 'none; rustc did the fast gates, so no lust codes below' })"
$lines += "- Runs per model and nugget: $Reps; budget: $Budget candidates; one job"
if ($skipped.Count -gt 0) { $lines += "- Skipped: $($skipped -join '; ')" }
$lines += ''
$lines += '## Models as loaded'
$lines += ''
$lines += '| Model | Size | On the GPU | Context | First load |'
$lines += '|---|---:|---:|---:|---:|'
foreach ($item in $loaded) {
    $note = ''
    if (-not $item.context_ok) { $note = " (too small: a prompt here can reach about $promptTokens tokens, and the results below are not to be trusted)" }
    $lines += "| $($item.model) | $($item.size_gb) GB | $($item.vram_gb) GB | $($item.context)$note | $($item.load_s) s |"
}
$lines += ''
$lines += '## Solved'
$lines += ''
$lines += 'Each cell: runs solved of runs made; then, over the solved runs, the median number of model candidates it took and the median wall time.'
$lines += ''
$passAll = @($Nuggets | Where-Object { $objective[$_] -eq 'pass_all' })
# minimize and fastest: no "solved", only a best proven score (lower is better).
$minimize = @($Nuggets | Where-Object { $objective[$_] -ne 'pass_all' })
$lines += '| Model | ' + ($passAll -join ' | ') + ' |'
$lines += '|---|' + (($passAll | ForEach-Object { '---' }) -join '|') + '|'
foreach ($item in $loaded) {
    $cells = @()
    foreach ($nugget in $passAll) {
        $runs = @($results | Where-Object { $_.model -eq $item.model -and $_.nugget -eq $nugget })
        $won = @($runs | Where-Object { $_.solved })
        $cell = "$($won.Count)/$($runs.Count)"
        if ($won.Count -gt 0) {
            $candidates = Get-Median ([double[]]@($won | ForEach-Object { $_.solved_at_candidate }))
            $seconds = Get-Median ([double[]]@($won | ForEach-Object { $_.wall_s }))
            $cell += "; $candidates cand.; $seconds s"
        }
        $cells += $cell
    }
    $lines += "| $($item.model) | " + ($cells -join ' | ') + ' |'
}
$lines += ''
$lines += '## Unsolved runs'
$lines += ''
$lines += '| Model | Nugget | Rep | Seed score | Best proven | Candidates | Wall | How it ended |'
$lines += '|---|---|---:|---:|---:|---:|---:|---|'
foreach ($run in @($results | Where-Object { $_.objective -eq 'pass_all' -and -not $_.solved })) {
    $lines += "| $($run.model) | $($run.nugget) | $($run.rep) | $($run.seed_score) | $($run.best_score) | $($run.candidates) | $($run.wall_s) s | $($run.stop) |"
}
if ($minimize.Count -gt 0) {
    $lines += ''
    $lines += '## Minimize nuggets'
    $lines += ''
    $lines += 'Lower is better. The seed score is where every run starts; the best proven score is what the run reached.'
    if ($minimize -contains 'delta-kernel') {
        $lines += ''
        $lines += 'For `delta-kernel` (bytes over eight firmware pairs; `docs/delta-kernels.md`): add-only 1530, strategy A 731, strategy B 860, best of A and B 626, optimal parse 625.'
    }
    $lines += ''
    $lines += '| Model | Nugget | Rep | Seed score | Best proven | Candidates | Wall | How it ended |'
    $lines += '|---|---|---:|---:|---:|---:|---:|---|'
    foreach ($run in @($results | Where-Object { $_.objective -ne 'pass_all' })) {
        $lines += "| $($run.model) | $($run.nugget) | $($run.rep) | $($run.seed_score) | $($run.best_score) | $($run.candidates) | $($run.wall_s) s | $($run.stop) |"
    }
}
$lines += ''
$lines += '## Where candidates were rejected'
$lines += ''
$lines += '| Model | Candidates | prefilter | lust check | build | run | oracle | diverged | Seconds per candidate |'
$lines += '|---|---:|---:|---:|---:|---:|---:|---:|---:|'
foreach ($item in $loaded) {
    $runs = @($results | Where-Object { $_.model -eq $item.model })
    $count = ($runs | ForEach-Object { $_.candidates } | Measure-Object -Sum).Sum
    $sum = @{ prefilter = 0; check = 0; build = 0; run = 0; oracle = 0 }
    foreach ($run in $runs) { foreach ($gate in @('prefilter', 'check', 'build', 'run', 'oracle')) { $sum[$gate] += $run.rejected[$gate] } }
    $diverged = ($runs | ForEach-Object { $_.diverged } | Measure-Object -Sum).Sum
    $wall = ($runs | ForEach-Object { $_.wall_s } | Measure-Object -Sum).Sum
    $per = $null
    if ($count -gt 0) { $per = [math]::Round($wall / $count, 1) }
    $lines += "| $($item.model) | $count | $($sum.prefilter) | $($sum.check) | $($sum.build) | $($sum.run) | $($sum.oracle) | $diverged | $per |"
}
$lines += ''
$lines += '## lust error codes, all models'
$lines += ''
$all = @{}
foreach ($run in $results) { foreach ($code in $run.codes.Keys) { $all[$code] = [int]$all[$code] + [int]$run.codes[$code] } }
if ($all.Count -eq 0) {
    $lines += 'None.'
} else {
    $lines += '| Code | Candidates |'
    $lines += '|---|---:|'
    foreach ($entry in ($all.GetEnumerator() | Sort-Object -Property Value -Descending)) { $lines += "| $($entry.Key) | $($entry.Value) |" }
}
$lines += ''
$lines += 'Every run keeps its journal, its candidates and any `divergences/` under this directory.'
Write-Text (Join-Path $OutDir 'summary.md') (($lines -join "`n") + "`n")

Write-Host ''
Write-Host "summary: $(Join-Path $OutDir 'summary.md')"
Write-Host "to send the results back: zip $OutDir, or commit it on a branch."
