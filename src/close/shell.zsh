# Loaded only by Flere's private startup wrapper. No user file is changed.
zmodload zsh/system 2>/dev/null || return
zmodload zsh/datetime 2>/dev/null || return
sysopen -rw -o cloexec,nonblock -u _flere_close_fd "$_flere_close_dir/wake" || return
typeset -g _flere_at_prompt=0
_flere_prompt() {
  local code=$?
  (( _flere_at_prompt == 0 )) && printf '\033]133;D;%d\007' "$code"
  printf '\033]133;A\007'
  _flere_at_prompt=1
}
_flere_executing() {
  _flere_at_prompt=0
  printf '\033]133;C\007'
}
precmd_functions+=(_flere_prompt)
preexec_functions+=(_flere_executing)
_flere_close_widget() {
  emulate -L zsh
  local byte token phase expiry pids reason
  sysread -i $_flere_close_fd -s 1 byte 2>/dev/null || return
  [[ -r $_flere_close_dir/request ]] || return
  {
    IFS= read -r token
    IFS= read -r phase
    IFS= read -r expiry
    IFS= read -r pids
  } < "$_flere_close_dir/request"
  [[ ${#token} == 32 && $token != *[^0-9a-f]* && $expiry == <-> ]] || return
  (( EPOCHSECONDS <= expiry )) || return
  [[ $phase == probe || $phase == commit ]] || return
  if (( !_flere_at_prompt )) || [[ $CONTEXT != start ]]; then
    reason='The shell is running a command'
  elif [[ -n $BUFFER || -n $PREBUFFER ]] || (( PENDING || KEYS_QUEUED_COUNT )); then
    reason='The shell has an unsubmitted command'
  elif (( ${#jobstates} )); then
    reason='The shell has running or stopped jobs'
  elif [[ -n $pids ]]; then
    reason='The shell process state changed'
  fi
  printf '{"token":"%s","phase":"%s","pid":%d,"reason":"%s","processes":[],"services":[]}' \
    "$token" "$phase" "$$" "$reason" > "$_flere_close_dir/reply"
  if [[ $phase == commit && -z $reason && -r $_flere_close_dir/request ]]; then
    _flere_close_exit=1
  fi
}
zle -N _flere_close_widget
_flere_close_handler() {
  local _flere_close_exit=0
  zle _flere_close_widget
  # Leave from an ordinary shell function, after the ZLE widget has unwound.
  (( _flere_close_exit )) && builtin exit
}
zle -F $_flere_close_fd _flere_close_handler
printf '%d\n' $$ > "$_flere_close_dir/ready"
