#include "quota_model.hpp"
#include "cJSON.h"
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <limits>
#include <memory>

namespace quota {
namespace {
using Json = std::unique_ptr<cJSON, decltype(&cJSON_Delete)>;
constexpr double exact_integer_max = 9007199254740991.0;
const cJSON* field(const cJSON* object, const char* key) { return cJSON_GetObjectItemCaseSensitive(object, key); }
bool nullable(const cJSON* value) { return !value || cJSON_IsNull(value); }
bool integer(const cJSON* value, double max = exact_integer_max) {
    return cJSON_IsNumber(value) && std::isfinite(value->valuedouble) && value->valuedouble >= 0 && value->valuedouble <= max && std::floor(value->valuedouble) == value->valuedouble;
}
bool text(const cJSON* value, Text& out, bool nonempty = true) {
    if (!cJSON_IsString(value) || !value->valuestring) return false;
    const size_t length = std::strlen(value->valuestring);
    if (length > 64 || (nonempty && !length)) return false;
    for (size_t i = 0; i < length; ++i) if (static_cast<unsigned char>(value->valuestring[i]) < 32) return false;
    std::memcpy(out.data(), value->valuestring, length + 1);
    return true;
}
bool optional_time(const cJSON* value, std::optional<int64_t>& out) {
    if (nullable(value)) { out.reset(); return true; }
    if (!integer(value)) return false;
    out = static_cast<int64_t>(value->valuedouble); return true;
}
bool optional_u32(const cJSON* value, std::optional<uint32_t>& out) {
    if (nullable(value)) { out.reset(); return true; }
    if (!integer(value, UINT32_MAX)) return false;
    out = static_cast<uint32_t>(value->valuedouble); return true;
}
bool optional_percent(const cJSON* value, std::optional<double>& out) {
    if (nullable(value)) { out.reset(); return true; }
    if (!cJSON_IsNumber(value) || !std::isfinite(value->valuedouble) || value->valuedouble < 0) return false;
    out = value->valuedouble; return true;
}
bool equals(const cJSON* value, const char* expected) { return cJSON_IsString(value) && value->valuestring && std::strcmp(value->valuestring, expected) == 0; }
bool valid_utf8(std::string_view input) {
    for (size_t i = 0; i < input.size(); ++i) {
        const auto lead = static_cast<unsigned char>(input[i]);
        if (lead < 0x80) continue;
        unsigned count;
        uint32_t code;
        if (lead >= 0xC2 && lead <= 0xDF) { count = 1; code = lead & 0x1F; }
        else if (lead >= 0xE0 && lead <= 0xEF) { count = 2; code = lead & 0x0F; }
        else if (lead >= 0xF0 && lead <= 0xF4) { count = 3; code = lead & 0x07; }
        else return false;
        if (i + count >= input.size()) return false;
        for (unsigned j = 0; j < count; ++j) {
            const auto next = static_cast<unsigned char>(input[++i]);
            if ((next & 0xC0) != 0x80) return false;
            code = (code << 6) | (next & 0x3F);
        }
        if ((count == 1 && code < 0x80) || (count == 2 && code < 0x800) || (count == 3 && code < 0x10000) || (code >= 0xD800 && code <= 0xDFFF) || code > 0x10FFFF) return false;
    }
    return true;
}
// Reject deep JSON before cJSON allocates its recursive tree. Escaped NUL cannot silently truncate names or values.
bool bounded_json(std::string_view input) {
    if (!valid_utf8(input)) return false;
    unsigned depth = 0;
    bool string = false;
    for (size_t i = 0; i < input.size(); ++i) {
        const unsigned char c = input[i];
        if (!c) return false;
        if (string) {
            if (c == '\\') {
                if (++i >= input.size()) return false;
                if (input[i] == 'u' && input.substr(i + 1, 4) == "0000") return false;
            } else if (c == '"') string = false;
            else if (c < 32) return false;
        } else if (c == '"') string = true;
        else if (c == '{' || c == '[') { if (++depth > 12) return false; }
        else if (c == '}' || c == ']') { if (!depth) return false; --depth; }
    }
    return !string && depth == 0;
}
bool unique_keys(const cJSON* node) {
    // ponytail: quadratic duplicate-key scan is bounded by 16 KiB; use a hash set if the protocol size grows.
    if (cJSON_IsObject(node)) {
        for (auto a = node->child; a; a = a->next)
            for (auto b = a->next; b; b = b->next)
                if (a->string && b->string && std::strcmp(a->string, b->string) == 0) return false;
    }
    for (auto child = node->child; child; child = child->next) if (!unique_keys(child)) return false;
    return true;
}
bool decode(const cJSON* root, Provider provider, Snapshot& out) {
    if (!cJSON_IsObject(root) || !integer(field(root,"schema_version"),1) || field(root,"schema_version")->valueint != 1) return false;
    if (!equals(field(root,"provider"), provider == Provider::Claude ? "claude" : "codex")) return false;
    out.provider = provider;
    const char* states[] = {"ok","stale","reauth_required","rate_limited","unavailable","error"};
    bool known = false;
    for (size_t i = 0; i < 6; ++i) if (equals(field(root,"state"), states[i])) { out.state = static_cast<State>(i); known = true; }
    if (!known) return false;
    if (equals(field(root,"source"),"mock")) out.mock = true;
    else if (field(root,"source") && !equals(field(root,"source"),"provider")) return false;
    if (!optional_time(field(root,"fetched_at"),out.fetched_at) || !integer(field(root,"published_at")) || !integer(field(root,"stale_after_seconds"))) return false;
    out.published_at = static_cast<int64_t>(field(root,"published_at")->valuedouble);
    out.stale_after_seconds = static_cast<uint64_t>(field(root,"stale_after_seconds")->valuedouble);
    const auto error = field(root,"error_code");
    if (!nullable(error)) {
        const char* codes[] = {"disabled","unauthenticated","auth_rejected","access_denied","rate_limited","network","timeout","invalid_response","process_unavailable","unsupported","cache","upstream"};
        known = false;
        for (const auto code : codes) if (equals(error,code)) known = true;
        if (!known || !text(error,out.error_code)) return false;
    }
    const auto windows = field(root,"windows");
    if (!cJSON_IsArray(windows) || cJSON_GetArraySize(windows) > static_cast<int>(max_windows)) return false;
    for (auto item = windows->child; item; item = item->next) {
        auto& window = out.windows[out.window_count++];
        if (!cJSON_IsObject(item) || !text(field(item,"limit_id"),window.limit_id) || !text(field(item,"key"),window.key) || !text(field(item,"label"),window.label) || !optional_u32(field(item,"duration_minutes"),window.duration_minutes) || !optional_percent(field(item,"used_percent"),window.used_percent) || !optional_percent(field(item,"remaining_percent"),window.remaining_percent) || !optional_time(field(item,"resets_at"),window.resets_at)) return false;
        if (window.duration_minutes && *window.duration_minutes == 0) return false;
        if (window.used_percent.has_value() != window.remaining_percent.has_value()) return false;
        if (window.used_percent && std::abs(*window.remaining_percent - std::clamp(100.0 - *window.used_percent,0.0,100.0)) > 0.000001) return false;
        for (size_t i = 0; i + 1 < out.window_count; ++i)
            if (out.windows[i].limit_id == window.limit_id && out.windows[i].key == window.key) return false;
    }
    const auto bank = field(root,"banked_resets");
    if (!nullable(bank)) {
        if (provider != Provider::Codex || !cJSON_IsObject(bank)) return false;
        out.banked_resets.emplace(); auto& credit = *out.banked_resets;
        if (!optional_u32(field(bank,"available_count"),credit.available_count) || !integer(field(bank,"known_nonexpiring_count"),UINT32_MAX)) return false;
        credit.known_nonexpiring_count = static_cast<uint32_t>(field(bank,"known_nonexpiring_count")->valuedouble);
        if (equals(field(bank,"details_status"),"complete")) credit.details_status = Details::Complete;
        else if (equals(field(bank,"details_status"),"partial")) credit.details_status = Details::Partial;
        else if (!equals(field(bank,"details_status"),"unavailable")) return false;
        const auto dates = field(bank,"known_expirations_at");
        if (!cJSON_IsArray(dates) || cJSON_GetArraySize(dates) > static_cast<int>(max_expirations)) return false;
        for (auto date = dates->child; date; date = date->next) {
            if (!integer(date)) return false;
            credit.expirations[credit.expiration_count++] = static_cast<int64_t>(date->valuedouble);
        }
    }
    return true;
}
size_t index(Provider provider) { return provider == Provider::Claude ? 0 : 1; }
}
void Clock::sync(int64_t utc, int64_t monotonic_ms) { if (utc >= 0 && monotonic_ms >= 0) { utc_ = utc; monotonic_ms_ = monotonic_ms; } }
void Clock::clear() { utc_.reset(); }
std::optional<int64_t> Clock::now(int64_t monotonic_ms) const {
    if (!utc_ || monotonic_ms < monotonic_ms_) return std::nullopt;
    const auto elapsed = (monotonic_ms - monotonic_ms_) / 1000;
    if (*utc_ > INT64_MAX - elapsed) return std::nullopt;
    return *utc_ + elapsed;
}
bool valid_timezone(std::string_view value) {
    if (value.empty() || value.size() > 64) return false;
    for (const char c : value)
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || std::strchr("+-:,.<>/", c))) return false;
    return true;
}
const char* layout_name(Layout layout) { return layout == Layout::Cards ? "cards" : layout == Layout::Gauges ? "gauges" : "terminal"; }
std::optional<Layout> layout_from_name(std::string_view name) {
    for (const auto layout : {Layout::Cards, Layout::Gauges, Layout::Terminal}) if (name == layout_name(layout)) return layout;
    return std::nullopt;
}
ApplyResult Model::ingest(std::string_view topic, std::string_view payload) {
    if (payload.empty() || payload.size() > max_snapshot_bytes || !bounded_json(payload)) return ApplyResult::Invalid;
    const char* end = nullptr;
    Json root(cJSON_ParseWithLengthOpts(payload.data(),payload.size(),&end,false),cJSON_Delete);
    if (!root || !end || !unique_keys(root.get())) return ApplyResult::Invalid;
    for (const char* p = end; p < payload.data() + payload.size(); ++p) if (*p != ' ' && *p != '\n' && *p != '\r' && *p != '\t') return ApplyResult::Invalid;
    if (topic == "ai-quota/v1/server/status") {
        if (!cJSON_IsObject(root.get())) return ApplyResult::Invalid;
        bool online = equals(field(root.get(),"state"),"online");
        if (!online && !equals(field(root.get(),"state"),"offline")) return ApplyResult::Invalid;
        if (server_online == online) return ApplyResult::Ignored;
        server_online = online; return ApplyResult::Applied;
    }
    if (topic == device_config_topic) {
        const auto version = field(root.get(), "schema_version");
        if (!cJSON_IsObject(root.get()) || !integer(version, 1) || version->valueint != 1) return ApplyResult::Invalid;
        // Per field: a bad or unknown field keeps the previous value, never rejects the payload.
        DeviceConfig next = config;
        const auto layout = field(root.get(), "layout");
        if (cJSON_IsString(layout) && layout->valuestring) if (const auto parsed = layout_from_name(layout->valuestring)) next.layout = *parsed;
        const auto timezone = field(root.get(), "timezone");
        if (cJSON_IsString(timezone) && timezone->valuestring && valid_timezone(timezone->valuestring)) {
            next.timezone = {};
            std::strcpy(next.timezone.data(), timezone->valuestring);
        }
        const auto brightness = field(root.get(), "brightness");
        if (cJSON_IsNumber(brightness) && std::isfinite(brightness->valuedouble) && std::floor(brightness->valuedouble) == brightness->valuedouble)
            next.brightness = static_cast<int>(std::clamp(brightness->valuedouble, 10.0, 100.0));
        if (next == config) return ApplyResult::Ignored;
        config = next; return ApplyResult::Applied;
    }
    Provider provider;
    if (topic == "ai-quota/v1/providers/claude") provider = Provider::Claude;
    else if (topic == "ai-quota/v1/providers/codex") provider = Provider::Codex;
    else return ApplyResult::Invalid;
    // Snapshot lives on heap: the MQTT callback stack is intentionally small on ESP32.
    auto next = std::make_unique<Snapshot>();
    if (!decode(root.get(),provider,*next)) return ApplyResult::Invalid;
    const auto i = index(provider);
    if (present_[i]) {
        const auto& old = snapshots_[i];
        if (!old.mock && next->mock) return ApplyResult::Ignored;
        // A live snapshot replaces DEMO outright; mock data must never become unlabeled live data.
        if (old.mock == next->mock) {
            if (next->published_at <= old.published_at) return ApplyResult::Ignored;
            if (old.fetched_at && next->fetched_at && *next->fetched_at < *old.fetched_at) return ApplyResult::Ignored;
            if (old.fetched_at && !next->fetched_at) {
                // An empty error update cannot erase the last successful data or its age.
                next->fetched_at = old.fetched_at; next->windows = old.windows;
                next->window_count = old.window_count; next->banked_resets = old.banked_resets;
            }
        }
    }
    snapshots_[i] = std::move(*next); present_[i] = true;
    return ApplyResult::Applied;
}
const Snapshot* Model::snapshot(Provider provider) const { const auto i = index(provider); return present_[i] ? &snapshots_[i] : nullptr; }
Freshness freshness(const Snapshot& snapshot, std::optional<int64_t> now) {
    Freshness result;
    if (!now || !snapshot.fetched_at) return result;
    if (*snapshot.fetched_at > *now) {
        result.clock_skew = *snapshot.fetched_at - *now > 60;
        result.stale = result.clock_skew; result.age_seconds = 0; return result;
    }
    result.age_seconds = *now - *snapshot.fetched_at;
    result.stale = static_cast<uint64_t>(*result.age_seconds) >= snapshot.stale_after_seconds;
    return result;
}
std::optional<int64_t> countdown(std::optional<int64_t> target, std::optional<int64_t> now) {
    if (!target || !now) return std::nullopt;
    return *target > *now ? *target - *now : 0;
}
std::string duration(int64_t seconds) {
    char buffer[48];
    seconds = std::max<int64_t>(0, seconds);
    const auto days = seconds / 86400;
    const auto hours = seconds / 3600 % 24;
    const auto minutes = seconds / 60 % 60;
    if (days) std::snprintf(buffer,sizeof buffer,"%lldd %02lldh",static_cast<long long>(days),static_cast<long long>(hours));
    else std::snprintf(buffer,sizeof buffer,"%02lld:%02lld:%02lld",static_cast<long long>(hours),static_cast<long long>(minutes),static_cast<long long>(seconds % 60));
    return buffer;
}
std::string timestamp(std::optional<int64_t> utc) {
    if (!utc) return "Unknown";
    const auto value = static_cast<time_t>(*utc);
    tm local{}; if (!localtime_r(&value,&local)) return "Unknown";
    char buffer[48]; if (!std::strftime(buffer,sizeof buffer,"%d %b %H:%M %Z",&local)) return "Unknown";
    return buffer;
}
std::string badge(const Snapshot* snapshot, const Model& model, std::optional<int64_t> now) {
    std::string result;
    if (!model.mqtt_connected || !model.server_online) result = "OFFLINE";
    if (!snapshot) return result.empty() ? "WAITING FOR DATA" : result + " / NO DATA";
    const char* states[] = {"OK","STALE","REAUTH REQUIRED","RATE LIMITED","UNAVAILABLE","ERROR"};
    if (!result.empty()) result += " / ";
    result += states[static_cast<size_t>(snapshot->state)];
    const auto fresh = freshness(*snapshot,now);
    if (fresh.stale && snapshot->state != State::Stale) result += " / STALE";
    if (fresh.clock_skew) result += " / CLOCK SKEW";
    if (snapshot->mock) result += " / DEMO";
    return result;
}
const char* provider_name(Provider provider) { return provider == Provider::Claude ? "Claude" : "Codex"; }
const char* details_name(Details details) { return details == Details::Complete ? "complete" : details == Details::Partial ? "partial" : "unavailable"; }
}
