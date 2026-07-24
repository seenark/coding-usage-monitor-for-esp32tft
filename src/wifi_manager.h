#pragma once

#include <Arduino.h>
#include <WiFi.h>

enum class WifiState { Offline, Connecting, Online };

struct WifiRetryConfig {
  uint32_t initial_delay_ms = 1000;
  uint32_t max_delay_ms = 30000;
};

struct WifiStatus {
  WifiState state = WifiState::Offline;
  uint32_t retry_delay_ms = 0;
  uint32_t next_retry_ms = 0;
  uint32_t attempt_count = 0;
};

class WifiManager {
 public:
  explicit WifiManager(WifiRetryConfig retry = {});

  void set_ssid(const char* ssid);
  void set_password(const char* password);

  void begin();
  void connect();
  void tick();
  void disconnect();

  WifiState state() const;
  WifiStatus status() const;

 private:
  void start_attempt(uint32_t now_ms);
  void schedule_retry(uint32_t now_ms);

  String ssid_;
  String password_;
  WifiRetryConfig retry_;
  WifiStatus status_;
};
