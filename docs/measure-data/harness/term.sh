#!/bin/bash
# types like a person: characters at 5-12 per second, a pause, sometimes a burst of command output
phr=("cd ~/project && git status" "grep -rn waydesk src | head" "make test" "vim notes.md" "ls -la /usr/share" "tail -n 20 build.log" "python3 analyse.py data.csv" "journalctl -u waydesk | tail")
while true; do
  p=${phr[RANDOM % ${#phr[@]}]}
  printf '$ '; for ((i=0;i<${#p};i++)); do printf '%s' "${p:i:1}"; sleep 0.0$((RANDOM%8+8)); done; echo
  case $((RANDOM%3)) in 0) ls -R /usr/share/doc 2>/dev/null | head -$((RANDOM%25+5));; 1) seq 1 $((RANDOM%30+10)) | sed 's/^/line /';; *) :;; esac
  sleep $((RANDOM%5+2))
done
