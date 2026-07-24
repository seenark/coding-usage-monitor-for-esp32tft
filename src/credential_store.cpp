#include "credential_store.h"

namespace {
constexpr const char *NVS_NAMESPACE = "codex_auth";
constexpr const char *ACCESS_KEY = "access";
constexpr const char *REFRESH_KEY = "refresh";
}

bool CredentialStore::begin() {
  if (open_) return true;
  open_ = preferences_.begin(NVS_NAMESPACE, false);
  return open_;
}

bool CredentialStore::save(const String &access_token, const String &refresh_token) {
  if (!open_ && !begin()) return false;
  if (access_token.isEmpty() || refresh_token.isEmpty()) return false;
  return preferences_.putString(ACCESS_KEY, access_token) > 0 &&
         preferences_.putString(REFRESH_KEY, refresh_token) > 0;
}

bool CredentialStore::load(String &access_token, String &refresh_token) {
  if (!open_ && !begin()) return false;
  access_token = preferences_.getString(ACCESS_KEY, "");
  refresh_token = preferences_.getString(REFRESH_KEY, "");
  return !access_token.isEmpty() && !refresh_token.isEmpty();
}

bool CredentialStore::has_credentials() {
  if (!open_ && !begin()) return false;
  return preferences_.isKey(ACCESS_KEY) && preferences_.isKey(REFRESH_KEY);
}

bool CredentialStore::erase() {
  if (!open_ && !begin()) return false;
  const bool removed_access = preferences_.remove(ACCESS_KEY);
  const bool removed_refresh = preferences_.remove(REFRESH_KEY);
  return removed_access || removed_refresh || !has_credentials();
}

void CredentialStore::end() {
  if (open_) preferences_.end();
  open_ = false;
}
