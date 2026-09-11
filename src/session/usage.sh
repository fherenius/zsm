# Portable host-side store shared by plugin instances. Records are timestamp,
# hex key, and optional hex value. Raw paths/names never enter shell source.
set -eu
umask 077
export LC_ALL=C
case "$1" in
    usage) store_dir="${XDG_CACHE_HOME:-$HOME/.cache}/zsm"; store_name=session-usage; limit=4096; fields=2 ;;
    projects) store_dir="${XDG_STATE_HOME:-$HOME/.local/state}/zsm"; store_name=projects; limit=0; fields=3 ;;
    *) echo 'Unknown ZSM store' >&2; exit 1 ;;
esac
shift
mkdir -p "$store_dir"
store_file="$store_dir/$store_name"
lock_dir="$store_file.lock"
# mkdir provides mutual exclusion on macOS and Linux without requiring flock.
# Never break another writer's lock: timeout rather than risk losing records.
attempt=0
until mkdir "$lock_dir" 2>/dev/null; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ]; then
        printf 'ZSM store is locked: %s (check for a stopped writer)\n' "$lock_dir" >&2
        exit 1
    fi
    sleep 0.05
done
trap 'rm -f "$lock_dir/input" "$lock_dir/ordered" "$lock_dir/sorted" "$lock_dir/next" "$lock_dir/pid"; rmdir "$lock_dir"' 0
trap 'exit 1' HUP INT TERM
printf '%s\n' "$$" > "$lock_dir/pid"
: > "$lock_dir/input"
if [ -f "$store_file" ]; then
    cat "$store_file" > "$lock_dir/input"
    printf '\n' >> "$lock_dir/input"
fi
if [ "$#" -gt 0 ]; then
    printf '%s\n' "$@" >> "$lock_dir/input"
fi
# Compare nanosecond timestamps as decimal strings, never floating-point
# numbers: adjacent visits routinely differ by less than awk's precision.
awk -v fields="$fields" '
    NF == fields && $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9a-f]+$/ && length($2) % 2 == 0 {
        if (NF == 3 && $3 != "-" && ($3 !~ /^[0-9a-f]+$/ || length($3) % 2 != 0)) next
        stamp = $1
        sub(/^0+/, "", stamp)
        if (stamp == "") stamp = "0"
        if (length(stamp) > 39 || (length(stamp) == 39 && "x" stamp > "x340282366920938463463374607431768211455")) next
        record = stamp " " $2 (NF == 3 ? " " $3 : "")
        key = "x" $2
        if (!(key in timestamps) || length(stamp) > length(timestamps[key]) ||
            (length(stamp) == length(timestamps[key]) && ("x" stamp > "x" timestamps[key] ||
             ("x" stamp == "x" timestamps[key] && "x" record > "x" records[key])))) {
            timestamps[key] = stamp
            records[key] = record
        }
    }
    END { for (key in records) print length(timestamps[key]), records[key] }
' "$lock_dir/input" > "$lock_dir/ordered"
sort -k1,1nr -k2,2r -k3,3 "$lock_dir/ordered" > "$lock_dir/sorted"
awk -v limit="$limit" 'limit == 0 || NR <= limit { sub(/^[0-9]+ /, ""); print }' "$lock_dir/sorted" > "$lock_dir/next"
# The rename and read happen under the lock, so readers see a complete snapshot
# and no concurrent append can be lost during compaction.
mv "$lock_dir/next" "$store_file"
cat "$store_file"
