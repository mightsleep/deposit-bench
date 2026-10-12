#!/bin/sh
# Median per benchmark from divan output files; first file is the baseline, the others get main / x.
awk '
FNR == 1 { file++ ; name[file] = FILENAME; sub(/.*\//, "", name[file]); sub(/-r[0-9]+\.txt$/, "", name[file]) }
{
  line = $0
  if (!match(line, /[├╰]─ /)) next
  pre = substr(line, 1, RSTART - 1); tmp = pre; gsub(/│/, "|", tmp); depth = int(length(tmp) / 3)
  rest = substr(line, RSTART + RLENGTH); split(rest, parts, /  +/); path[depth] = parts[1]
  for (k = depth + 1; k < 10; k++) delete path[k]
  n = 0; s = rest
  while (match(s, /[0-9.]+ (ns|µs|ms|s) /)) { v[++n] = substr(s, RSTART, RLENGTH - 1); s = substr(s, RSTART + RLENGTH) }
  if (n < 3) next
  p = ""; for (k = 0; k <= depth; k++) if (k in path) p = p "/" path[k]
  split(v[3], tu, " "); t = tu[1]; if (tu[2] == "ns") t /= 1000; else if (tu[2] == "ms") t *= 1000; else if (tu[2] == "s") t *= 1e6
  med[p, file] = t; keys[p] = 1
}
END {
  printf "%-46s %9s", "benchmark", name[1]; for (f = 2; f <= file; f++) printf " %9s", name[f]; print ""
  for (p in keys) {
    printf "%-46s %9.2f", p, med[p, 1]
    for (f = 2; f <= file; f++) printf " %8.2fx", (med[p, f] > 0 ? med[p, 1] / med[p, f] : 0)
    print ""
  }
}' "$@" | sort
