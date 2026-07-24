#include "codex_auth.h"

namespace codex_auth {

bool DevicePrompt::valid() const {
  return verification_uri.length() > 0 && user_code.length() > 0 &&
         expires_in_seconds > 0;
}

bool TokenRecord::has_access_token() const {
  return access_token.length() > 0;
}

bool TokenRecord::expired(uint32_t now_ms) const {
  return !has_access_token() ||
         (access_token_expires_at_ms != 0 &&
          static_cast<int32_t>(now_ms - access_token_expires_at_ms) >= 0);
}

bool AuthError::ok() const {
  return kind == AuthErrorKind::None;
}

bool AuthResult::ok() const {
  return error.ok() && state != AuthState::Failed;
}

AuthResult AuthResult::failure(AuthErrorKind kind,
                              const String &redacted_message) {
  AuthResult result;
  result.state = AuthState::Failed;
  result.error.kind = kind;
  result.error.redacted_message = redacted_message;
  return result;
}

AuthResult accept_device_prompt(const DevicePrompt &prompt) {
  if (!prompt.valid()) {
    return AuthResult::failure(AuthErrorKind::InvalidInput,
                               String("invalid device prompt"));
  }
  AuthResult result;
  result.state = AuthState::DevicePromptReady;
  return result;
}

AuthResult mark_waiting_for_approval() {
  AuthResult result;
  result.state = AuthState::WaitingForApproval;
  return result;
}

AuthResult mark_authenticated(const TokenRecord &record) {
  if (!record.has_access_token()) {
    return AuthResult::failure(AuthErrorKind::InvalidInput,
                               String("missing access token"));
  }
  AuthResult result;
  result.state = AuthState::Authenticated;
  return result;
}

AuthResult mark_failure(AuthErrorKind kind, const String &redacted_message) {
  return AuthResult::failure(kind, redacted_message);
}

AuthResult refresh_if_needed(TokenRecord &record, uint32_t now_ms) {
  if (!record.has_access_token()) {
    return AuthResult::failure(AuthErrorKind::InvalidInput,
                               String("missing access token"));
  }
  if (!record.expired(now_ms)) {
    AuthResult result;
    result.state = AuthState::Authenticated;
    return result;
  }
  return AuthResult::failure(AuthErrorKind::Unsupported,
                             String("refresh transport unavailable"));
}

AuthResult logout(TokenRecord &record) {
  record.access_token = "";
  record.refresh_token = "";
  record.access_token_expires_at_ms = 0;
  record.issued_at_ms = 0;
  AuthResult result;
  result.state = AuthState::Idle;
  return result;
}

}  // namespace codex_auth
