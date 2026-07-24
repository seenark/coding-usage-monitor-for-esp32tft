#pragma once

#include <Arduino.h>
#include <WiFi.h>
#include <Preferences.h>

class WifiConfigPortal {
 public:
  void begin();
  void tick();
  bool has_credentials() const;
  const String &ssid() const;
  const String &password() const;

 private:
  void handle_client(WiFiClient &client);
  static String decode_form_value(const String &value);

  WiFiServer server_{80};
  Preferences preferences_;
  String ssid_;
  String password_;
  bool portal_started_ = false;
};
