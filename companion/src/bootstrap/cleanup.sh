set -eu
export LC_ALL=C
directory=$1
if [ ! -e "$directory" ] && [ ! -L "$directory" ]; then exit 0; fi
if [ ! -d "$directory" ] || [ -L "$directory" ]; then
    echo 'Bootstrap cleanup directory changed; retained for inspection' >&2; exit 2
fi
case $(uname -s) in
    Linux) identity=$(/usr/bin/stat -c '%u %a' "$directory") ;;
    Darwin) identity=$(/usr/bin/stat -f '%u %Lp' "$directory") ;;
    *) exit 2;;
esac
set -- $identity
if [ "$#" -ne 2 ] || [ "$1" != "$(id -u)" ] || [ "$((0$2 & 077))" -ne 0 ]; then
    echo 'Bootstrap cleanup ownership changed; retained for inspection' >&2; exit 2
fi
rm -f "$directory/manifest.json" "$directory/flere"
rmdir "$directory"
