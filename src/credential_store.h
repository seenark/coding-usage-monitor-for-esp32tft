#pragma once

#include <Arduino.h>
#include <Preferences.h>

// NVS boundary only. Encryption/provisioning remains a release-stage concern.
class CredentialStore {
 public:
  bool begin();
  bool save(const String &access_token, const String &refresh_token);
  bool load(String &access_token, String &refresh_token);
  bool has_credentials();
  bool erase();
  void end();

 private:
  Preferences preferences_;
  bool open_ = false;
};
