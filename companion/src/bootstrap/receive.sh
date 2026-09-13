set -eu
export LC_ALL=C
umask 077
cache=$1
manifest_bytes=$2
payload_bytes=$3
expected_sha256=$4
case $cache in /*) ;; *) echo 'Bootstrap cache must be absolute' >&2; exit 2;; esac
case $(uname -s) in
    Linux) metadata() { /usr/bin/stat -c '%u %a' "$1"; }
           digest() { /usr/bin/sha256sum < "$1"; } ;;
    Darwin) metadata() { /usr/bin/stat -f '%u %Lp' "$1"; }
            digest() { /usr/bin/shasum -a 256 < "$1"; } ;;
    *) echo 'Unsupported remote bootstrap platform' >&2; exit 2;;
esac
if [ ! -e "$cache" ] && [ ! -L "$cache" ]; then mkdir -p -m 700 "$cache"; fi
if [ ! -d "$cache" ] || [ -L "$cache" ]; then
    echo 'Bootstrap cache is not a private regular directory' >&2; exit 2
fi
set -- $(metadata "$cache")
if [ "$#" -ne 2 ] || [ "$1" != "$(id -u)" ] || [ "$((0$2 & 077))" -ne 0 ]; then
    echo 'Bootstrap cache must be private and user-owned' >&2; exit 2
fi
directory=$(mktemp -d "$cache/bootstrap.XXXXXXXXXX")
cleanup() {
    rm -f "$directory/manifest.json" "$directory/flere"
    rmdir "$directory" 2>/dev/null || :
}
trap cleanup 0
trap 'exit 1' 1 2 15
# BSD head may read past its requested prefix on a pipe. The small manifest
# needs byte-sized reads so none of the following executable is consumed.
dd bs=1 count="$manifest_bytes" 2>/dev/null > "$directory/manifest.json"
head -c "$((payload_bytes + 1))" > "$directory/flere"
if [ "$(wc -c < "$directory/manifest.json" | tr -d ' ')" != "$manifest_bytes" ] ||
   [ "$(wc -c < "$directory/flere" | tr -d ' ')" != "$payload_bytes" ]; then
    echo 'Bootstrap transfer length differs from the verified package' >&2; exit 2
fi
actual=$(digest "$directory/flere")
actual=${actual%% *}
if [ "$actual" != "$expected_sha256" ]; then
    echo 'Bootstrap payload SHA-256 differs from the verified package' >&2; exit 2
fi
chmod 700 "$directory/flere"
printf 'FLERE-UPLOAD-1\n'
printf '%s' "$directory" | od -An -v -tx1 | tr -d ' \n'
printf '\nEND\n'
trap - 0 1 2 15
