# Private startup wrapper. Keep the user's startup file and prompt hooks intact.
[[ -r "$HOME/.bashrc" ]] && source "$HOME/.bashrc"
_flere_mark_prompt() {
  local code=$?
  printf '\033]133;A\007'
  return "$code"
}
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a '* ]]; then
  PROMPT_COMMAND+=(_flere_mark_prompt)
else
  PROMPT_COMMAND="${PROMPT_COMMAND-}"$'\n_flere_mark_prompt'
fi
