# Run on the host: WASI's /data directory is private to each plugin instance.
# Each visit is a single append, so concurrent instances cannot overwrite one
# another's history. Rust merges by timestamp even if commands finish out of order.
set -eu
umask 077
usage_dir="${XDG_CACHE_HOME:-$HOME/.cache}/zsm"
mkdir -p "$usage_dir"
usage_file="$usage_dir/session-usage"
if [ "$#" -gt 0 ]; then
    printf '%s\n' "$@" >> "$usage_file"
fi
if [ -f "$usage_file" ]; then
    cat "$usage_file"
fi
