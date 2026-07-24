#include "wifi_manager.h"

#include <algorithm>

WifiManager::WifiManager(WifiRetryConfig retry) : retry_(retry) {
  if (retry_.max_delay_ms < retry_.initial_delay_ms) {
    retry_.max_delay_ms = retry_.initial_delay_ms;
  }
}

void WifiManager::set_ssid(const char* ssid) {
  ssid_ = ssid == nullptr ? "" : ssid;
}

void WifiManager::set_password(const char* password) {
  password_ = password == nullptr ? "" : password;
}

void WifiManager::begin() {
  status_ = {};
  status_.state = WifiState::Connecting;
  start_attempt(millis());
}

void WifiManager::connect() {
  if (status_.state == WifiState::Online || status_.state == WifiState::Connecting) {
    return;
  }
  status_.state = WifiState::Connecting;
  start_attempt(millis());
}

void WifiManager::tick() {
  const uint32_t now_ms = millis();
  const wl_status_t link = WiFi.status();

  if (link == WL_CONNECTED) {
    status_.state = WifiState::Online;
    status_.retry_delay_ms = 0;
    status_.next_retry_ms = 0;
    return;
  }

  if (status_.state == WifiState::Online) {
    status_.state = WifiState::Connecting;
    schedule_retry(now_ms);
  }

  if (status_.state != WifiState::Connecting || status_.next_retry_ms == 0 ||
      static_cast<int32_t>(now_ms - status_.next_retry_ms) >= 0) {
    if (status_.state == WifiState::Connecting &&
        (status_.next_retry_ms == 0 || static_cast<int32_t>(now_ms - status_.next_retry_ms) >= 0)) {
      start_attempt(now_ms);
    }
  }
}

void WifiManager::disconnect() {
  WiFi.disconnect();
  status_.state = WifiState::Offline;
  status_.retry_delay_ms = 0;
  status_.next_retry_ms = 0;
}

WifiState WifiManager::state() const { return status_.state; }

WifiStatus WifiManager::status() const { return status_; }

void WifiManager::start_attempt(uint32_t now_ms) {
  if (ssid_.isEmpty()) {
    status_.state = WifiState::Offline;
    status_.next_retry_ms = 0;
    return;
  }

  WiFi.begin(ssid_.c_str(), password_.c_str());
  ++status_.attempt_count;
  status_.state = WiFi.status() == WL_CONNECTED ? WifiState::Online : WifiState::Connecting;

  if (status_.state == WifiState::Online) {
    status_.retry_delay_ms = 0;
    status_.next_retry_ms = 0;
    return;
  }
  schedule_retry(now_ms);
}

void WifiManager::schedule_retry(uint32_t now_ms) {
  uint32_t delay_ms = status_.retry_delay_ms;
  if (delay_ms == 0) {
    delay_ms = retry_.initial_delay_ms;
  } else if (delay_ms > retry_.max_delay_ms / 2) {
    delay_ms = retry_.max_delay_ms;
  } else {
    delay_ms *= 2;
  }
  status_.retry_delay_ms = std::min(delay_ms, retry_.max_delay_ms);
  status_.next_retry_ms = now_ms + status_.retry_delay_ms;
}
