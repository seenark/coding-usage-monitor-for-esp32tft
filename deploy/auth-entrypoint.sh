#!/bin/sh
set -eu
umask 077
# No API-billing routes, externally supplied tokens, or inferred model calls.
unset OPENAI_API_KEY ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_OAUTH_TOKEN
export RUST_LOG=off DISABLE_AUTOUPDATER=1 DISABLE_TELEMETRY=1 DISABLE_ERROR_REPORTING=1
codex_status() {
  # Codex's API-key status exposes partial key material. Capture and discard all output.
  if result=$(codex -c 'cli_auth_credentials_store="file"' -c 'forced_login_method="chatgpt"' login status 2>&1); then
    case "$result" in 'Logged in using ChatGPT') return 0 ;; esac
  fi
  printf '%s\n' 'Codex subscription authentication is unavailable.' >&2
  return 1
}
claude_status() {
  status_file=$(mktemp /tmp/quota-auth.XXXXXX)
  trap 'rm -f "$status_file"' EXIT
  if claude auth status >"$status_file" 2>/dev/null && \
      python3 /usr/local/lib/check-claude-status.py "$status_file"; then
    rm -f "$status_file"
    trap - EXIT
    return 0
  fi
  rm -f "$status_file"
  trap - EXIT
  printf '%s\n' 'Claude.ai subscription authentication is unavailable.' >&2
  return 1
}
[ "$#" -eq 1 ] || exit 2
case "$1" in
  codex-login)
    codex -c 'cli_auth_credentials_store="file"' -c 'forced_login_method="chatgpt"' login --device-auth
    codex_status
    ;;
  codex-status) codex_status ;;
  claude-login)
    # Default auth login uses Claude subscriptions. Never pass --console.
    claude auth login
    claude_status
    ;;
  claude-status) claude_status ;;
  *) printf '%s\n' 'Use codex-login, codex-status, claude-login, or claude-status.' >&2; exit 2 ;;
esac
printf '%s\n' 'Subscription authentication verified.'
