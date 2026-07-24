#pragma once

#include <Arduino.h>

// Experimental/private-endpoint boundary. This module deliberately performs no
// network I/O and must not be used as a production authentication client.
namespace codex_auth {

enum class AuthState : uint8_t {
  Idle,
  DevicePromptReady,
  WaitingForApproval,
  Authenticated,
  Expired,
  Failed,
};

// Values needed to display a device flow prompt. The endpoint and user code
// are transient protocol data; callers must not persist or log them.
struct DevicePrompt {
  String verification_uri;
  String user_code;
  uint32_t expires_in_seconds = 0;
  uint32_t interval_seconds = 5;

  bool valid() const;
};

// Experimental token state. Secret fields are kept only for a future secure
// storage implementation; this boundary never prints, transmits, or persists
// them.
struct TokenRecord {
  String access_token;
  String refresh_token;
  uint32_t access_token_expires_at_ms = 0;
  uint32_t issued_at_ms = 0;

  bool has_access_token() const;
  bool expired(uint32_t now_ms) const;
};

enum class AuthErrorKind : uint8_t {
  None,
  InvalidInput,
  Rejected,
  Expired,
  StorageUnavailable,
  Unsupported,
  Transport,
  Unknown,
};

// Deliberately redacted error: details must be safe for UI/status reporting.
struct AuthError {
  AuthErrorKind kind = AuthErrorKind::None;
  String redacted_message;

  bool ok() const;
};

// A transport-neutral result helper. No success constructor performs auth.
struct AuthResult {
  AuthState state = AuthState::Idle;
  AuthError error;

  bool ok() const;
  static AuthResult failure(AuthErrorKind kind, const String &redacted_message);
};

// State-only boundary operations. They validate/transition local state and do
// not contact Codex or any private endpoint.
AuthResult accept_device_prompt(const DevicePrompt &prompt);
AuthResult mark_waiting_for_approval();
AuthResult mark_authenticated(const TokenRecord &record);
AuthResult mark_failure(AuthErrorKind kind, const String &redacted_message);
// Local-only lifecycle helpers. Network refresh/revoke must be supplied by a
// secure transport adapter; these functions never contact an endpoint.
AuthResult refresh_if_needed(TokenRecord &record, uint32_t now_ms);
AuthResult logout(TokenRecord &record);

}  // namespace codex_auth
