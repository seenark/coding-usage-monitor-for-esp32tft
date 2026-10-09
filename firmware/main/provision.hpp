#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string_view>

namespace quota::device {
constexpr size_t kMaxFormBytes = 28672;
struct Credentials {
    char ssid[33]{};
    char wifi_password[65]{};
    char mqtt_host[254]{};
    char mqtt_username[129]{};
    char mqtt_password[257]{};
    char ca[8193]{};
    char token[33]{};
    uint16_t mqtt_port = 0;
    bool tls = false;
};

// All fields are required, even when their value is empty. No values are logged.
bool parse_provisioning(std::string_view body, Credentials& result);
void clear_sensitive(void* memory, size_t size);
// Just-submitted setup values that failed to join Wi-Fi this many times are discarded and setup restarts.
constexpr int kUnsavedJoinAttempts = 3;
// Saved values are never abandoned: they keep retrying with backoff.
bool abandon_unsaved_setup(bool persisted, int failed_attempts);
// Returns the host-order IPv4 address embedded in an IPv4-mapped IPv6 address
// (::ffff:a.b.c.d), or nothing for any other IPv6 address.
std::optional<uint32_t> mapped_ipv4(const uint8_t (&address)[16]);
} // namespace quota::device
