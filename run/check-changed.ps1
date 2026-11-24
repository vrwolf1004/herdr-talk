param([string]$Exe, [string]$Src)
$e = Get-Item -LiteralPath $Exe -ErrorAction SilentlyContinue
if (-not $e) { 'CHANGED'; exit 0 }
$files = $Src -split ',' | ForEach-Object { Get-ChildItem -LiteralPath $_ -Recurse -File -ErrorAction SilentlyContinue }
$latest = ($files | Measure-Object LastWriteTime -Maximum).Maximum
if ($latest -and $latest -gt $e.LastWriteTime) { 'CHANGED' } else { 'SAME' }