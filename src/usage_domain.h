#pragma once
#include <Arduino.h>

enum class Connectivity { Offline, Connecting, Online };

struct UsageState {
  String plan = "Pro";
  uint8_t five_hour_used = 76;
  uint8_t weekly_used = 48;
  uint32_t five_hour_reset_seconds = 2 * 60 * 60 + 14 * 60;
  Connectivity connectivity = Connectivity::Online;
  uint32_t last_updated_ms = 0;
};

inline UsageState mock_usage() {
  UsageState usage;
  usage.last_updated_ms = millis() - 12000;
  return usage;
}

class UsageProvider {
 public:
  virtual ~UsageProvider() = default;
  virtual bool refresh(UsageState& usage) = 0;
};

class MockUsageProvider final : public UsageProvider {
 public:
  bool refresh(UsageState& usage) override {
    usage = mock_usage();
    return true;
  }
};
