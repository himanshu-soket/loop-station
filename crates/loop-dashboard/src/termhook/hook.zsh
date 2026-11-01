# loop-terminal.zsh — Loop terminal observability hook library.
#
# Installed by `loop install-terminal`. Captures command metadata (command
# text, start/end timestamps, duration, exit code, cwd, shell info) and
# reports it to the local Loop dashboard. Never blocks the prompt, never
# writes to the terminal, and fails silently when the dashboard is down.
#
# Output capture is opt-in only: wrap a command with `loopterm` to have its
# stdout+stderr stored alongside the trace. Ordinary commands are recorded
# without output.

# ---------------------------------------------------------------------
# Globals (names prefixed _LOOP_TERM_ to avoid collisions)
# ---------------------------------------------------------------------

typeset -g _LOOP_TERM_ENABLED=1
typeset -g _LOOP_TERM_CMD_ID=''
typeset -g _LOOP_TERM_CMD_START_MS=0
typeset -g _LOOP_TERM_PREEXEC_CMD=''
typeset -g _LOOP_TERM_IN_HOOK=0
typeset -g _LOOP_TERM_NESTED=0

# Toggle helpers: `loop-terminal-off` / `loop-terminal-on`.
loop-terminal-off() { _LOOP_TERM_ENABLED=0; }
loop-terminal-on()  { _LOOP_TERM_ENABLED=1; }

# ---------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------

# Current time in unix milliseconds using zsh's built-in EPOCHREALTIME
# (no external process). Falls back to EPOCHSECONDS on ancient zsh.
_loop_terminal_now_ms() {
  local t=${EPOCHREALTIME:-}
  if [[ -n $t && ${#t} -ge 14 ]]; then
    t=${t//./}
    printf '%s' "${t:0:13}"
  else
    printf '%s' $(( ${EPOCHSECONDS:-$(date +%s)} * 1000 ))
  fi
}

# UUID v4 trace id: prefer uuidgen, fall back to $RANDOM-derived hex shaped
# as a version-4 UUID. Uniqueness within a session is what matters here.
_loop_terminal_new_id() {
  if (( $+commands[uuidgen] )); then
    local u
    u=$(uuidgen 2>/dev/null) || u=''
    if [[ -n $u ]]; then
      printf '%s' "${u:l}"
      return
    fi
  fi
  local hex=''
  while [[ ${#hex} -lt 32 ]]; do
    hex+=$(printf '%04x' $(( RANDOM % 65536 )))
  done
  local y
  case $(( RANDOM % 4 )) in
    0) y=8;; 1) y=9;; 2) y=a;; 3) y=b;;
  esac
  printf '%s-%s-4%s-%s%s-%s' \
    "${hex:0:8}" "${hex:8:4}" "${hex:12:3}" "$y" "${hex:16:3}" "${hex:19:13}"
}

# Escape a string for embedding inside a JSON string literal.
#
# Accepts the value either as `$1` (preferred) or on stdin, because call
# sites use both `func "$value"` and `printf '%s' "$v" | func`.
_loop_terminal_json_escape() {
  local s=${1:-$(cat)}
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\n'/\\n}
  s=${s//$'\r'/\\r}
  s=${s//$'\t'/\\t}
  # Strip any remaining control characters.
  s=${s//[[:cntrl:]]/}
  printf '"%s"' "${s:0:500}"
}

# Redact obvious secrets from a command line before it leaves the machine.
#
# Implementation notes: zsh's `${var#pat#repl}` is NOT a substitution form
# (it is prefix-removal), and the `##`/`#` quantifiers require
# EXTENDED_GLOB which we must not impose on the user's shell. This
# implementation therefore uses only portable constructs: word splitting
# via ${(z)...}, `case` glob matching, and `${var//pat/repl}` with plain
# character classes. The server re-scrubs everything as defense in depth.
_loop_terminal_scrub() {
  local -a words out
  local skip=0 w
  # ${(z)...} splits exactly the way zsh would parse the command line, and
  # ${(Q)...} then removes one level of quoting so quoted values are seen
  # as single words. The server re-scrubs everything as defense in depth.
  words=(${(Q)${(z)1}})
  for w in "${words[@]}"; do
    if (( skip )); then
      out+=("<redacted>")
      skip=0
      continue
    fi
    case $w in
      # --token <value> (and friends): redact the NEXT word
      (--token|--password|--passwd|--secret|--api-key|--api_key|--apikey|\
       --access-key|--access_key|--authorization|--auth)
        out+=("$w"); skip=1; continue ;;
      # --token=<value>
      (--token=*|--password=*|--passwd=*|--secret=*|--api-key=*|--api_key=*|\
       --apikey=*|--access-key=*|--access_key=*|--authorization=*|--auth=*)
        out+=("${w%%=*}=<redacted>"); continue ;;
    esac
    case $w in
      # NAME=value where NAME mentions key/token/secret/pass/credential
      (*[Kk][Ee][Yy]=*|*[Tt][Oo][Kk][Ee][Nn]=*|*[Ss][Ee][Cc][Rr][Ee][Tt]=*|\
       *[Pp][Aa][Ss][Ss]*=*|*[Cc][Rr][Ee][Dd]*=*)
        out+=("${w%%=*}=<redacted>"); continue ;;
    esac
    # A word that is exactly "Bearer"/"bearer", or ends with ":Bearer" (as
    # in `Authorization:Bearer`), introduces a credential in the NEXT word
    # once ${(z)} splitting has separated the two.
    case $w in
      ([Bb]earer|*:[Bb]earer) out+=("$w"); skip=1; continue ;;
    esac
    # Inline "Bearer <creds>" inside one word (e.g. `Authorization: Bearer x`
    # after quote removal leaves it as a single word).
    w=${w//[Bb][Ee][Aa][Rr][Ee][Rr][ ][A-Za-z0-9._~+=-]*/Bearer <redacted>}
    # URL userinfo scheme://user:pass@… → scheme://<redacted>@…
    if [[ "$w" == *://* ]]; then
      local _pre=${w%%://*} _rest=${w#*://}
      if [[ "$_rest" == *@* && "${_rest%%@*}" == *:* ]]; then
        w="$_pre://<redacted>@${_rest#*@}"
      fi
    fi
    out+=("$w")
  done
  printf '%s' "${out[*]}"
}

# Read port + token from the dashboard config (~/.loop/dashboard/loop.json,
# or $LOOP_DASHBOARD_DIR/loop.json when that variable is exported).
# Sets _LOOP_TERM_BASE and _LOOP_TERM_TOKEN on success.
_loop_terminal_endpoint() {
  local cfg="${LOOP_DASHBOARD_DIR:-$HOME/.loop/dashboard}/loop.json"
  [[ -r $cfg ]] || return 1
  local port token
  token=$(sed -n 's/.*"token":[[:space:]]*"\([^"]*\)".*/\1/p' "$cfg" 2>/dev/null | head -n 1)
  port=$(sed -n 's/.*"port":[[:space:]]*\([0-9]\{1,5\}\).*/\1/p' "$cfg" 2>/dev/null | head -n 1)
  [[ -n $token ]] || return 1
  if [[ -n $port && $port -ge 1 && $port -le 65535 ]]; then
    _LOOP_TERM_BASE="http://127.0.0.1:$port"
    _LOOP_TERM_TOKEN=$token
    return 0
  fi
  return 1
}

# POST one JSON payload. Runs detached so the prompt is never blocked.
#
# NOTE: the parameter is deliberately named `uri`, not `path`: in zsh,
# `$path` is a special array tied to `$PATH`, and a `local path=…` inside
# a function would clobber PATH for everything the function runs.
_loop_terminal_post() {
  local uri=$1 body=$2
  (
    command curl -s -o /dev/null --max-time 2 --connect-timeout 1 \
      -X POST "$_LOOP_TERM_BASE$uri" \
      -H "Authorization: Bearer $_LOOP_TERM_TOKEN" \
      -H 'Content-Type: application/json' \
      --data "$body" </dev/null >/dev/null 2>&1
  ) &!
  return 0
}

# Report a completed command. Arguments:
#   $1 trace id   $2 scrubbed command   $3 exit code
#   $4 start ms   $5 end ms   $6 output json (empty = none)
_loop_terminal_report() {
  local id=$1 cmd=$2 rc=$3 start_ms=$4 end_ms=$5 out_json=${6:-}
  command -v curl >/dev/null 2>&1 || return 0
  _loop_terminal_endpoint || return 0
  local host_name
  host_name=$(hostname 2>/dev/null) || host_name=localhost
  local payload
  payload="{\"traceId\":$(printf '%s' "$id" | _loop_terminal_json_escape),\
\"command\":$(printf '%s' "$cmd" | _loop_terminal_json_escape),\
\"startedAtMs\":$start_ms,\
\"endedAtMs\":$end_ms,\
\"exitCode\":$rc,\
\"cwd\":$(printf '%s' "$PWD" | _loop_terminal_json_escape),\
\"host\":$(printf '%s' "$host_name" | _loop_terminal_json_escape),\
\"shell\":\"zsh\",\
\"shellPid\":$$,\
\"sessionId\":\"terminal:$$\""
  if [[ -n $out_json ]]; then
    payload="$payload,\"output\":$out_json}"
  else
    payload="$payload}"
  fi
  _loop_terminal_post '/api/terminal/events' "$payload"
}

# ---------------------------------------------------------------------
# Hook entry points
# ---------------------------------------------------------------------

_loop_terminal_preexec() {
  # Save $? before anything can clobber it.
  typeset -g _LOOP_TERM_PREV_RC=$?
  [[ -n ${LOOP_TERM_DISABLE:-} ]] && return 0
  (( _LOOP_TERM_ENABLED )) || return 0
  (( _LOOP_TERM_IN_HOOK )) && return 0
  _LOOP_TERM_IN_HOOK=1

  local cmd=${1:-}
  # Skip our own plumbing and non-commands.
  case $cmd in
    (_loop_terminal*|loop-terminal-*) _LOOP_TERM_IN_HOOK=0; return 0 ;;
    ('') _LOOP_TERM_IN_HOOK=0; return 0 ;;
  esac

  _LOOP_TERM_PREEXEC_CMD=$cmd
  _LOOP_TERM_CMD_START_MS=$(_loop_terminal_now_ms)
  _LOOP_TERM_CMD_ID=$(_loop_terminal_new_id)
  _LOOP_TERM_IN_HOOK=0
  return 0
}

_loop_terminal_precmd() {
  # Capture the exit status of the user's command FIRST — every statement
  # after this must preserve it for the prompt.
  local rc=$?
  (( _LOOP_TERM_ENABLED )) || return 0
  (( _LOOP_TERM_IN_HOOK )) && return 0
  local id=$_LOOP_TERM_CMD_ID
  local start_ms=$_LOOP_TERM_CMD_START_MS
  _LOOP_TERM_CMD_ID=''
  _LOOP_TERM_CMD_START_MS=0
  [[ -n $id && $start_ms -gt 0 ]] || return 0
  local cmd
  cmd=$(_loop_terminal_scrub "$_LOOP_TERM_PREEXEC_CMD")
  [[ -n $cmd ]] || return 0
  local end_ms
  end_ms=$(_loop_terminal_now_ms)
  _LOOP_TERM_IN_HOOK=1
  _loop_terminal_report "$id" "$cmd" "$rc" "$start_ms" "$end_ms"
  _LOOP_TERM_IN_HOOK=0
  return 0
}

# ---------------------------------------------------------------------
# loopterm: opt-in output capture wrapper
# ---------------------------------------------------------------------

# Wrap a command to capture its stdout+stderr (size-capped) and store it on
# the same trace as the command itself. The command still owns the terminal
# interactively unless output capture forces redirection — interactive TUIs
# should not be wrapped.
loopterm() {
  if (( _LOOP_TERM_ENABLED == 0 )) || (( _LOOP_TERM_NESTED )); then
    "$@"
    return $?
  fi
  _LOOP_TERM_NESTED=1
  local rc=0 tmp='' out=''
  # Adopt the trace id preexec created for this command line.
  local id=${_LOOP_TERM_CMD_ID:-}
  if [[ -z $id ]]; then
    id=$(_loop_terminal_new_id)
  fi
  if command -v mktemp >/dev/null 2>&1; then
    tmp=$(mktemp "${TMPDIR:-/tmp}/loopterm.XXXXXX" 2>/dev/null) || tmp=''
  fi
  if [[ -n $tmp ]]; then
    "$@" >"$tmp" 2>&1
    rc=$?
    out=$(head -c 65536 "$tmp" 2>/dev/null)
    command rm -f "$tmp" 2>/dev/null
    if [[ -n $out ]]; then
      print -r -- "$out"
    fi
  else
    "$@"
    rc=$?
  fi
  _LOOP_TERM_NESTED=0
  # Report output on the same trace id (before precmd completes it).
  if command -v curl >/dev/null 2>&1 && _loop_terminal_endpoint; then
    local host_name
    host_name=$(hostname 2>/dev/null) || host_name=localhost
    local payload
    payload="{\"traceId\":$(printf '%s' "$id" | _loop_terminal_json_escape),\
\"output\":$(printf '%s' "$out" | _loop_terminal_json_escape),\
\"truncated\":false,\
\"command\":$(printf '%s' "${_LOOP_TERM_PREEXEC_CMD:-$*}" | _loop_terminal_scrub | _loop_terminal_json_escape),\
\"startedAtMs\":$_LOOP_TERM_CMD_START_MS,\
\"cwd\":$(printf '%s' "$PWD" | _loop_terminal_json_escape),\
\"host\":$(printf '%s' "$host_name" | _loop_terminal_json_escape),\
\"shell\":\"zsh\",\
\"shellPid\":$$,\
\"sessionId\":\"terminal:$$\"}"
    _loop_terminal_post '/api/terminal/output' "$payload"
  fi
  return $rc
}

# ---------------------------------------------------------------------
# Registration (idempotent — safe to source repeatedly)
# ---------------------------------------------------------------------

if (( ! ${+_LOOP_TERM_HOOKS_INSTALLED} )); then
  typeset -g _LOOP_TERM_HOOKS_INSTALLED=1
  # Append to the standard hook arrays; existing user hooks are preserved.
  [[ -z "${preexec_functions[(re)_loop_terminal_preexec]:-}" ]] && \
    preexec_functions+=(_loop_terminal_preexec)
  [[ -z "${precmd_functions[(re)_loop_terminal_precmd]:-}" ]] && \
    precmd_functions+=(_loop_terminal_precmd)
fi

return 0
