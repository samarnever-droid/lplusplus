#!/usr/bin/env sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
python3 "$ROOT/safety/generate_list_game_stress.py" >/dev/null
FILE="$ROOT/safety/generated/list_game_stress_10k.lpp"
[ "$(wc -l < "$FILE")" -ge 10000 ]
grep -Fq 'def labyrinth_room_1669' "$FILE"
grep -Fq 'list_push(tiles, seed)' "$FILE"
grep -Fq 'write_file("lpp_list_labyrinth_save.txt"' "$FILE"
grep -Fq 'struct Player:' "$FILE"
echo "PASS generated 10k L++ list game stress asset"
