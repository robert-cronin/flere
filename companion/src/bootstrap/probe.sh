set -eu
export LC_ALL=C
command_name=$1
legacy_name=$2
state_name=$3
cache_name=$4
explicit_executable=$5
explicit_state=$6
case ${HOME-} in /*) ;; *) echo 'Remote HOME must be absolute' >&2; exit 2;; esac
state_base=${XDG_STATE_HOME:-"$HOME/.local/state"}
data_base=${XDG_DATA_HOME:-"$HOME/.local/share"}
cache_base=${XDG_CACHE_HOME:-"$HOME/.cache"}
for directory in "$state_base" "$data_base" "$cache_base"; do
    case $directory in /*) ;; *) echo 'Remote XDG directories must be absolute' >&2; exit 2;; esac
done
present() { [ -e "$1" ] || [ -L "$1" ]; }
if [ -n "$explicit_state" ]; then
    state=$explicit_state
else
    state="$state_base/$state_name"
fi
case $state in /*) ;; *) echo 'Remote --state must be absolute' >&2; exit 2;; esac
state_presence=missing
if present "$state"; then
    state_presence=directory
    if [ ! -d "$state" ] || [ -L "$state" ]; then state_presence=unsafe; fi
fi
endpoint=absent
if present "$state/control.sock"; then endpoint=present; fi
receipt=absent
root="$data_base/$state_name/install"
if present "$root/flere.json" || present "$root/flere.pending.json"; then receipt=present; fi
executable=
resolve() {
    case $1 in
        /*) if [ -f "$1" ] && [ -x "$1" ]; then executable=$1; fi ;;
        */*) echo 'Remote executable must be absolute or one command name' >&2; exit 2 ;;
        *) executable=$(command -v "$1" 2>/dev/null || :)
           case $executable in /*) ;; *) executable=;; esac ;;
    esac
}
if [ -n "$explicit_executable" ]; then
    resolve "$explicit_executable"
else
    for candidate in "$HOME/.local/bin/$command_name" "$command_name"; do
        resolve "$candidate"
        if [ -n "$executable" ]; then break; fi
    done
fi
selected_executable=$executable
executable=
resolve "$HOME/.local/bin/$legacy_name"
if [ -z "$executable" ]; then resolve "$legacy_name"; fi
legacy_executable=$executable
executable=$selected_executable
field() {
    printf '%s\t' "$1"
    printf '%s' "$2" | od -An -v -tx1 | tr -d ' \n'
    printf '\n'
}
printf 'FLERE-BOOTSTRAP-1\n'
field os "$(uname -s)"
field arch "$(uname -m)"
field home "$HOME"
field cache "$cache_base/$cache_name/bootstrap"
field state "$state"
field state_presence "$state_presence"
field endpoint "$endpoint"
field receipt "$receipt"
field executable "$executable"
field legacy_executable "$legacy_executable"
printf 'END\n'
