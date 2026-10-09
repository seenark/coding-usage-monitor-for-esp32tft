#pragma once
#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

namespace quota {
constexpr size_t max_snapshot_bytes = 16384;
constexpr size_t max_windows = 32;
constexpr size_t max_expirations = 64;
using Text = std::array<char, 65>;
enum class Provider { Claude, Codex };
enum class State { Ok, Stale, ReauthRequired, RateLimited, Unavailable, Error };
enum class Details { Complete, Partial, Unavailable };
enum class ApplyResult { Applied, Ignored, Invalid };
struct Window {
    Text limit_id{}, key{}, label{};
    std::optional<uint32_t> duration_minutes;
    std::optional<double> used_percent, remaining_percent;
    std::optional<int64_t> resets_at;
};
struct BankedResets {
    std::optional<uint32_t> available_count;
    Details details_status = Details::Unavailable;
    std::array<int64_t, max_expirations> expirations{};
    size_t expiration_count = 0;
    uint32_t known_nonexpiring_count = 0;
};
struct Snapshot {
    Provider provider = Provider::Claude;
    State state = State::Unavailable;
    bool mock = false;
    std::optional<int64_t> fetched_at;
    int64_t published_at = 0;
    uint64_t stale_after_seconds = 0;
    std::array<Window, max_windows> windows{};
    size_t window_count = 0;
    std::optional<BankedResets> banked_resets;
    Text error_code{};
};
class Clock {
public:
    void sync(int64_t utc, int64_t monotonic_ms);
    void clear();
    std::optional<int64_t> now(int64_t monotonic_ms) const;
private:
    std::optional<int64_t> utc_;
    int64_t monotonic_ms_ = 0;
};
// Display layout, brightness and timezone pushed by the server on device/config.
enum class Layout { Cards, Gauges, Terminal };
struct DeviceConfig {
    Layout layout = Layout::Cards;
    Text timezone{'I', 'C', 'T', '-', '7'};
    int brightness = 80;
    bool operator==(const DeviceConfig& other) const {
        return layout == other.layout && timezone == other.timezone && brightness == other.brightness;
    }
};
constexpr const char* device_config_topic = "ai-quota/v1/device/config";
// POSIX TZ string: 1..64 characters from [A-Za-z0-9+-:,./<>].
bool valid_timezone(std::string_view value);
const char* layout_name(Layout);
std::optional<Layout> layout_from_name(std::string_view);
class Model {
public:
    ApplyResult ingest(std::string_view topic, std::string_view payload);
    const Snapshot* snapshot(Provider provider) const;
    bool server_online = false;
    bool mqtt_connected = false;
    // Updated only by device/config; each field falls back to its previous value on any problem.
    DeviceConfig config;
private:
    std::array<Snapshot, 2> snapshots_{};
    std::array<bool, 2> present_{};
};
struct Freshness {
    bool stale = false;
    bool clock_skew = false;
    std::optional<int64_t> age_seconds;
};
Freshness freshness(const Snapshot&, std::optional<int64_t> now);
std::optional<int64_t> countdown(std::optional<int64_t> target, std::optional<int64_t> now);
std::string duration(int64_t seconds);
std::string timestamp(std::optional<int64_t> utc);
std::string badge(const Snapshot*, const Model&, std::optional<int64_t> now);
const char* provider_name(Provider);
const char* details_name(Details);
}
