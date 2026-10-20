#!/bin/bash
# Runs civ6opt natively on Windows, from WSL, in a hidden window that keeps running when WSL or a terminal closes
# (the searches crash under WSL but not on Windows; see docs/history.md, 10/20).
#
#   optimizer/scripts/win_run.sh start <job> <civ6opt command and arguments>   start a job (returns at once)
#   optimizer/scripts/win_run.sh status <job>                                  the job's last log lines; "done" when finished
#   optimizer/scripts/win_run.sh fetch <job>                                   copy the job's --out file back
#
# Every argument that names an existing file is copied into the job's folder on Windows
# (C:\Users\<user>\civ6opt\jobs\<job>) and passed by its file name; --out is written there and fetch copies it back
# to the path given. The Windows build is made from the current source on every start.
set -eu
here=$PWD
cd "$(dirname "$0")/.."
WINUSER=${WINUSER:-nobus}
ROOT=/mnt/c/Users/$WINUSER/civ6opt
cmd=$1; job=$2; shift 2
J=$ROOT/jobs/$job
case $cmd in
start)
  [ -e "$J" ] && { echo "job $job exists: $J" >&2; exit 1; }
  nice -n 19 cargo build --release --target x86_64-pc-windows-gnu -j 2 2>&1 | grep -E '^error' -A5 && exit 1
  mkdir -p "$J"
  cp target/x86_64-pc-windows-gnu/release/civ6opt.exe "$J/"
  args=(); out=""; prev=""
  for a in "$@"; do
    f=$a; [[ $a == /* ]] || f=$here/$a
    if [ "$prev" = "--out" ]; then
      out=$(realpath -m "$f"); args+=("$(basename "$a")")
    elif [ -f "$f" ]; then
      cp "$f" "$J/"; args+=("$(basename "$a")")
    else
      args+=("$a")
    fi
    prev=$a
  done
  echo "$out" > "$J/out_path.txt"
  # PowerShell single-quoted strings: a quote is written twice
  ps_args=$(printf "'%s'," "${args[@]//\'/\'\'}"); ps_args=${ps_args%,}
  cat > "$J/run.ps1" <<EOF
Set-Location \$PSScriptRoot
"start \$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" | Out-File status.txt -Encoding utf8
\$p = Start-Process -FilePath (Join-Path \$PSScriptRoot 'civ6opt.exe') -ArgumentList @($ps_args) -NoNewWindow -PassThru -Wait \`
  -RedirectStandardOutput stdout.txt -RedirectStandardError stderr.txt
"done exit=\$(\$p.ExitCode) \$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" | Out-File status.txt -Append -Encoding utf8
EOF
  winpath="C:\\Users\\$WINUSER\\civ6opt\\jobs\\$job\\run.ps1"
  (cd "$J" && powershell.exe -NoProfile -Command "Start-Process powershell.exe -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','$winpath' -WindowStyle Hidden")
  echo "started $job in $J"
  ;;
status)
  tr -d '\r' < "$J/status.txt"
  tail -n "${LINES:-3}" "$J/stderr.txt" 2>/dev/null | tr -d '\r'
  tail -n 1 "$J/stdout.txt" 2>/dev/null | tr -d '\r'
  ;;
fetch)
  out=$(cat "$J/out_path.txt")
  [ -n "$out" ] || { echo "job $job has no --out" >&2; exit 1; }
  mkdir -p "$(dirname "$out")"
  cp "$J/$(basename "$out")" "$out"
  tr -d '\r' < "$J/stderr.txt" > "$out.log"
  echo "fetched $out (and $out.log)"
  ;;
*)
  echo "unknown command $cmd" >&2; exit 1
  ;;
esac
