#!/bin/sh
# table.sh main.txt a.txt ...: the divan median of each case per file, then
# main's median over each other file's
for f in "$@"; do
  sed -e 's/│/|/g; s/├─/+-/g; s/╰─/+-/g; s/µs/us/g' "$f" > "$f.a"
done
printf '%-58s' case
for f in "$@"; do n=$(basename "$f" .txt); printf ' %9s' "${n%-r*}"; done
echo
awk -F'|' '
function us(v,  n) { n = v + 0; if (v ~ /ns/) return n / 1000; if (v ~ /ms/) return n * 1000; return n }
FNR == 1 { f++ }
/^\+- / { grp = $1; sub(/^\+- /, "", grp); gsub(/ +$/, "", grp) }
/^\|  \+- / {
  name = $2; sub(/^  \+- /, "", name); sub(/ +[0-9.]+ [num]s *$/, "", name)
  key = grp " " name; m[f, key] = us($4); if (f == 1) order[++k] = key
}
END {
  for (i = 1; i <= k; i++) {
    key = order[i]; printf "%-58s", key
    for (j = 1; j <= f; j++) printf " %9.2f", m[j, key]
    for (j = 2; j <= f; j++) printf " %6.2fx", m[1, key] / m[j, key]
    printf "\n"
  }
}' $(for f in "$@"; do echo "$f.a"; done)
