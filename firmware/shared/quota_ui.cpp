#include "quota_ui.hpp"
#include <algorithm>
#include <cctype>
#include <cstdio>
#include <vector>
#include <cstring>
#include <ctime>

namespace quota {
namespace {
constexpr uint32_t kBackground = 0x0E1116;
constexpr uint32_t kCard = 0x1A1F27;
constexpr uint32_t kCardPressed = 0x262D38;
constexpr uint32_t kTrack = 0x2C3440;
constexpr uint32_t kText = 0xE8EDF2;
constexpr uint32_t kMuted = 0x8D99A6;
constexpr uint32_t kGood = 0x3FB950;
constexpr uint32_t kWarn = 0xE3A008;
constexpr uint32_t kBad = 0xF0554D;
constexpr uint32_t kOff = 0x5B6673;
constexpr lv_coord_t kTopBar = 30;

uint32_t accent(Provider provider) { return provider == Provider::Claude ? 0xE07A5F : 0x6AA8FF; }

// Plain layout container: no theme styles, never steals taps from its card.
lv_obj_t* box(lv_obj_t* parent, lv_flex_flow_t flow, lv_coord_t gap = 0) {
    auto object = lv_obj_create(parent);
    lv_obj_remove_style_all(object);
    lv_obj_clear_flag(object, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_add_flag(object, LV_OBJ_FLAG_EVENT_BUBBLE);
    lv_obj_set_size(object, LV_PCT(100), LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(object, flow);
    lv_obj_set_style_pad_row(object, gap, 0);
    lv_obj_set_style_pad_column(object, gap, 0);
    return object;
}

lv_obj_t* card(lv_obj_t* parent) {
    auto object = box(parent, LV_FLEX_FLOW_COLUMN, 3);
    lv_obj_set_style_bg_opa(object, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(object, lv_color_hex(kCard), 0);
    lv_obj_set_style_bg_color(object, lv_color_hex(kCardPressed), LV_STATE_PRESSED);
    lv_obj_set_style_radius(object, 10, 0);
    lv_obj_set_style_pad_all(object, 8, 0);
    return object;
}

lv_obj_t* text(lv_obj_t* parent, const std::string& value, const lv_font_t* font, uint32_t color, bool wrap = false) {
    auto object = lv_label_create(parent);
    lv_label_set_text(object, value.c_str());
    lv_obj_set_style_text_font(object, font, 0);
    lv_obj_set_style_text_color(object, lv_color_hex(color), 0);
    lv_obj_add_flag(object, LV_OBJ_FLAG_EVENT_BUBBLE);
    if (wrap) {
        lv_obj_set_width(object, LV_PCT(100));
        lv_label_set_long_mode(object, LV_LABEL_LONG_WRAP);
    }
    return object;
}

lv_obj_t* pill(lv_obj_t* parent, const char* value, uint32_t color) {
    auto object = text(parent, value, &lv_font_montserrat_12, kBackground);
    lv_obj_set_style_bg_opa(object, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(object, lv_color_hex(color), 0);
    lv_obj_set_style_radius(object, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_pad_hor(object, 7, 0);
    lv_obj_set_style_pad_ver(object, 1, 0);
    return object;
}

void meter(lv_obj_t* parent, const Window& window, uint32_t color) {
    auto bar = lv_bar_create(parent);
    lv_obj_clear_flag(bar, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_add_flag(bar, LV_OBJ_FLAG_EVENT_BUBBLE);
    lv_obj_set_size(bar, LV_PCT(100), 6);
    lv_bar_set_range(bar, 0, 1000);
    lv_bar_set_value(bar, window.remaining_percent ? static_cast<int32_t>(*window.remaining_percent * 10) : 0, LV_ANIM_OFF);
    lv_obj_set_style_bg_color(bar, lv_color_hex(kTrack), 0);
    lv_obj_set_style_bg_opa(bar, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(bar, lv_color_hex(color), LV_PART_INDICATOR);
    lv_obj_set_style_radius(bar, 3, 0);
    lv_obj_set_style_radius(bar, 3, LV_PART_INDICATOR);
}

uint32_t health(const Window& window) {
    if (!window.remaining_percent) return kOff;
    if (*window.remaining_percent >= 50) return kGood;
    return *window.remaining_percent >= 20 ? kWarn : kBad;
}

std::string percent_left(const Window& window) {
    if (!window.remaining_percent) return "--";
    char value[16];
    std::snprintf(value, sizeof value, "%.0f%%", *window.remaining_percent);
    return value;
}

// Minute precision: the screen redraws once a minute, not every second.
std::string short_duration(int64_t seconds) {
    if (seconds < 60) return "<1m";
    const auto days = seconds / 86400, hours = seconds / 3600 % 24, minutes = seconds / 60 % 60;
    char value[32];
    if (days) std::snprintf(value, sizeof value, "%lldd %lldh", static_cast<long long>(days), static_cast<long long>(hours));
    else if (hours) std::snprintf(value, sizeof value, "%lldh %lldm", static_cast<long long>(hours), static_cast<long long>(minutes));
    else std::snprintf(value, sizeof value, "%lldm", static_cast<long long>(minutes));
    return value;
}

std::string local_time(int64_t utc, std::optional<int64_t> now, const char* fallback_format = "%d %b %H:%M") {
    const auto value = static_cast<time_t>(utc);
    tm local{};
    if (!localtime_r(&value, &local)) return "?";
    // Within the coming week a weekday reads faster than a date.
    const bool this_week = now && utc >= *now && utc - *now < 6 * 86400;
    char result[32];
    if (!std::strftime(result, sizeof result, this_week ? "%a %H:%M" : fallback_format, &local)) return "?";
    return result;
}

std::string reset_line(const Window& window, std::optional<int64_t> now, bool with_time) {
    if (!window.resets_at) return "Reset time not reported";
    if (!now) return "Syncing clock...";
    const auto left = *window.resets_at - *now;
    // Never assume the quota refilled: wait for the provider to say so.
    if (left <= 0) return "Waiting for refresh";
    auto line = "Resets in " + short_duration(left);
    if (with_time) line += " (" + local_time(*window.resets_at, now) + ")";
    return line;
}

struct Status {
    const char* label;
    uint32_t color;
};

Status status(const Snapshot* snapshot, const Model& model, std::optional<int64_t> now) {
    if (!model.mqtt_connected) return {"Offline", kOff};
    if (!snapshot) return {"Waiting", kOff};
    switch (snapshot->state) {
    case State::Ok: {
        const auto fresh = freshness(*snapshot, now);
        if (fresh.clock_skew) return {"Clock?", kWarn};
        if (fresh.stale) return {"Stale", kWarn};
        return {"Live", kGood};
    }
    case State::Stale: return {"Stale", kWarn};
    case State::ReauthRequired: return {"Sign in", kBad};
    case State::RateLimited: return {"Limited", kWarn};
    case State::Unavailable: return {std::strcmp(snapshot->error_code.data(), "disabled") == 0 ? "Off" : "Unavailable", kOff};
    case State::Error: return {"Error", kBad};
    }
    return {"Unknown", kOff};
}

// One plain sentence for a person standing in front of the display.
const char* problem(const Snapshot& snapshot) {
    const std::string_view code = snapshot.error_code.data();
    if (code.empty()) return nullptr;
    if (code == "disabled") return "Turned off on the server.";
    if (code == "unauthenticated" || code == "auth_rejected") return "Sign in again on the server.";
    if (code == "access_denied") return "This account can't read usage.";
    if (code == "rate_limited") return "Provider is rate limiting. Retrying later.";
    if (code == "network" || code == "timeout" || code == "upstream") return "Can't reach the provider. Retrying.";
    if (code == "process_unavailable") return "Codex helper isn't running on the server.";
    if (code == "invalid_response") return "Provider sent data the server didn't understand.";
    return "Server reported a problem.";
}

std::string updated_line(const Snapshot& snapshot, std::optional<int64_t> now) {
    if (!snapshot.fetched_at) return "Never updated";
    const auto fresh = freshness(snapshot, now);
    if (fresh.age_seconds && !fresh.clock_skew) return "Updated " + short_duration(*fresh.age_seconds) + " ago";
    return "Updated " + local_time(*snapshot.fetched_at, std::nullopt);
}

// The short (usually 5-hour) and weekly windows; otherwise the first two.
std::array<const Window*, 2> overview_windows(const Snapshot& snapshot) {
    std::array<const Window*, 2> picked{};
    for (size_t i = 0; i < snapshot.window_count; ++i) {
        const auto& window = snapshot.windows[i];
        if (!window.duration_minutes) continue;
        if (*window.duration_minutes < 10080 && !picked[0]) picked[0] = &window;
        else if (*window.duration_minutes == 10080 && !picked[1]) picked[1] = &window;
    }
    for (size_t i = 0; i < snapshot.window_count && (!picked[0] || !picked[1]); ++i) {
        const auto* window = &snapshot.windows[i];
        if (window == picked[0] || window == picked[1]) continue;
        (picked[0] ? picked[1] : picked[0]) = window;
    }
    return picked;
}

bool any_mock(const Model& model) {
    for (auto provider : {Provider::Claude, Provider::Codex})
        if (auto snapshot = model.snapshot(provider); snapshot && snapshot->mock) return true;
    return false;
}
void card_header(lv_obj_t* panel, Provider provider, const Status& state) {
    auto header = box(panel, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(header, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_bottom(header, 4, 0);
    text(header, provider_name(provider), &lv_font_montserrat_16, accent(provider));
    pill(header, state.label, state.color);
}

// Terminal layout: the five-hour window is "5h", other labels are lowercased and clipped.
std::string short_label(const Window& window) {
    std::string value(window.label.data());
    if (value == "5-hour") return "5h";
    for (auto& c : value) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return value.substr(0, 14);
}

std::string compact_reset(const Window& window, std::optional<int64_t> now) {
    if (!window.resets_at) return "reset ?";
    if (!now) return "clock ?";
    const auto left = *window.resets_at - *now;
    return left <= 0 ? "awaiting refresh" : "resets " + short_duration(left);
}

// Terminal lines color segments with LVGL recolor ("#RRGGBB text#"); values never contain '#'.
std::string colored(uint32_t color, const std::string& value) {
    char prefix[12];
    std::snprintf(prefix, sizeof prefix, "#%06X ", static_cast<unsigned>(color));
    return prefix + value + "#";
}

lv_obj_t* mono(lv_obj_t* parent, const std::string& value) {
    auto object = text(parent, value, &lv_font_unscii_8, kText, true);
    lv_label_set_recolor(object, true);
    return object;
}
}

void Ui::create() {
    root_ = lv_obj_create(nullptr);
    lv_obj_set_style_bg_color(root_, lv_color_hex(kBackground), 0);
    lv_obj_set_style_text_color(root_, lv_color_hex(kText), 0);
    lv_obj_set_style_text_font(root_, &lv_font_montserrat_14, 0);
    lv_obj_set_style_pad_all(root_, 0, 0);
    lv_obj_clear_flag(root_, LV_OBJ_FLAG_SCROLLABLE);
    lv_scr_load(root_);
    render(std::nullopt);
}

void Ui::tick(int64_t monotonic_ms) {
    if (!root_) return;
    if (pending_) { selected_ = *pending_; confirm_open_ = false; pending_.reset(); last_tick_second_ = -1; }
    if (pending_action_) {
        switch (*pending_action_) {
        case OpenConfirm: confirm_open_ = true; break;
        case CloseConfirm: confirm_open_ = false; break;
        case Erase: confirm_open_ = false; reconfigure_requested_ = true; break;
        }
        pending_action_.reset();
        last_tick_second_ = -1;
    }
    const auto second = monotonic_ms / 1000;
    if (second == last_tick_second_) return;
    last_tick_second_ = second;
    const auto now = clock_.now(monotonic_ms);
    auto key = state_key(now);
    if (key != key_) { key_ = std::move(key); render(now); }
}

void Ui::setup(const char* ssid, const char* password, const char* address) {
    if (ssid && password && address) {
        setup_ssid_ = ssid; setup_password_ = password; setup_address_ = address;
        pending_ = Settings;
    } else {
        setup_ssid_.clear(); setup_password_.clear(); setup_address_.clear();
        if (selected_ == Settings) pending_ = Overview;
    }
    last_tick_second_ = -1;
}

void Ui::settings(const char* value) { settings_text_ = value ? value : ""; }

void Ui::notice(const char* value) { notice_ = value ? value : ""; }

bool Ui::take_reconfigure_request() { return std::exchange(reconfigure_requested_, false); }

std::string Ui::no_window_message(const Snapshot* snapshot) const {
    if (!snapshot) return notice_.empty() ? "Waiting for data from the server." : notice_;
    const char* message = problem(*snapshot);
    return message ? message : "No quota reported.";
}

void Ui::on_click(lv_event_t* event) {
    auto self = static_cast<Ui*>(lv_event_get_user_data(event));
    auto target = lv_event_get_current_target(event);
    self->pending_ = static_cast<Screen>(reinterpret_cast<uintptr_t>(lv_obj_get_user_data(target)));
}

void Ui::on_action(lv_event_t* event) {
    auto self = static_cast<Ui*>(lv_event_get_user_data(event));
    auto target = lv_event_get_current_target(event);
    self->pending_action_ = static_cast<Action>(reinterpret_cast<uintptr_t>(lv_obj_get_user_data(target)));
}

lv_obj_t* Ui::tap_target(lv_obj_t* object, Screen target) {
    lv_obj_add_flag(object, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_clear_flag(object, LV_OBJ_FLAG_EVENT_BUBBLE);
    lv_obj_set_user_data(object, reinterpret_cast<void*>(static_cast<uintptr_t>(target)));
    lv_obj_add_event_cb(object, on_click, LV_EVENT_CLICKED, this);
    return object;
}

lv_obj_t* Ui::action_target(lv_obj_t* object, Action action) {
    lv_obj_add_flag(object, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_clear_flag(object, LV_OBJ_FLAG_EVENT_BUBBLE);
    lv_obj_set_user_data(object, reinterpret_cast<void*>(static_cast<uintptr_t>(action)));
    lv_obj_add_event_cb(object, on_action, LV_EVENT_CLICKED, this);
    return object;
}

// Everything that can change what is drawn. Countdowns are minute precision.
std::string Ui::state_key(std::optional<int64_t> now) const {
    char part[96];
    std::snprintf(part, sizeof part, "%u|%lld|%d%d|%d%d|", static_cast<unsigned>(selected_),
        now ? static_cast<long long>(*now / 60) : -1LL, model_.mqtt_connected, model_.server_online,
        confirm_open_, static_cast<int>(model_.config.layout));
    std::string key = part;
    key += model_.config.timezone.data();
    key += '|';
    for (auto provider : {Provider::Claude, Provider::Codex}) {
        const auto snapshot = model_.snapshot(provider);
        if (!snapshot) { key += "-|"; continue; }
        std::snprintf(part, sizeof part, "%d|%lld|%lld|%d|", static_cast<int>(snapshot->state),
            static_cast<long long>(snapshot->published_at), static_cast<long long>(snapshot->fetched_at.value_or(-1)), snapshot->mock);
        key += part;
    }
    return key + settings_text_ + "|" + notice_ + "|" + setup_password_;
}

void Ui::render(std::optional<int64_t> now) {
    const lv_coord_t scroll = body_ && rendered_ == selected_ ? lv_obj_get_scroll_y(body_) : 0;
    lv_obj_clean(root_);
    body_ = nullptr;
    top_bar(now);
    body_ = lv_obj_create(root_);
    lv_obj_remove_style_all(body_);
    lv_obj_set_pos(body_, 0, kTopBar);
    lv_obj_set_size(body_, lv_disp_get_hor_res(nullptr), lv_disp_get_ver_res(nullptr) - kTopBar);
    lv_obj_set_style_pad_all(body_, 6, 0);
    lv_obj_set_style_pad_top(body_, 0, 0);
    lv_obj_set_style_pad_row(body_, 6, 0);
    lv_obj_set_style_pad_column(body_, 6, 0);
    lv_obj_set_scroll_dir(body_, LV_DIR_VER);
    lv_obj_set_scrollbar_mode(body_, LV_SCROLLBAR_MODE_ACTIVE);
    switch (selected_) {
    case Overview: overview(now); break;
    case ClaudeDetail: detail(Provider::Claude, now); break;
    case CodexDetail: detail(Provider::Codex, now); break;
    case Settings: settings_screen(); break;
    }
    lv_obj_update_layout(body_);
    if (scroll) lv_obj_scroll_to_y(body_, scroll, LV_ANIM_OFF);
    if (confirm_open_ && selected_ == Settings) confirm_dialog();
    rendered_ = selected_;
}

void Ui::top_bar(std::optional<int64_t> now) {
    // The bar itself is not a flex row: a content-sized flex group clipped the
    // status dot and clock, so the right side is aligned explicitly.
    auto bar = lv_obj_create(root_);
    lv_obj_remove_style_all(bar);
    lv_obj_clear_flag(bar, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(bar, LV_PCT(100), kTopBar);

    auto left = box(bar, LV_FLEX_FLOW_ROW, 6);
    lv_obj_set_size(left, LV_SIZE_CONTENT, kTopBar);
    lv_obj_align(left, LV_ALIGN_LEFT_MID, 8, 0);
    lv_obj_set_width(left, LV_SIZE_CONTENT);
    lv_obj_set_flex_align(left, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    if (selected_ == Overview) {
        text(left, "AI Quota", &lv_font_montserrat_16, kText);
    } else {
        // Large tap area for a finger on a 2.8" panel.
        auto back = tap_target(box(left, LV_FLEX_FLOW_ROW, 4), Overview);
        lv_obj_set_size(back, LV_SIZE_CONTENT, kTopBar);
        lv_obj_set_flex_align(back, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
        text(back, LV_SYMBOL_LEFT, &lv_font_montserrat_16, kMuted);
        const char* title = selected_ == ClaudeDetail ? "Claude" : selected_ == CodexDetail ? "Codex" : "Settings";
        const uint32_t color = selected_ == ClaudeDetail ? accent(Provider::Claude) : selected_ == CodexDetail ? accent(Provider::Codex) : kText;
        text(back, title, &lv_font_montserrat_16, color);
    }
    if (any_mock(model_)) pill(left, "DEMO", kWarn);

    const bool gear = selected_ == Overview;
    if (gear) {
        // 36x30 touch area for a fingertip.
        auto button = tap_target(box(bar, LV_FLEX_FLOW_ROW), Settings);
        lv_obj_set_size(button, 36, kTopBar);
        lv_obj_align(button, LV_ALIGN_RIGHT_MID, 0, 0);
        lv_obj_set_flex_align(button, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
        text(button, LV_SYMBOL_SETTINGS, &lv_font_montserrat_16, kMuted);
    }
    char clock[16] = "--:--";
    if (now) {
        const auto value = static_cast<time_t>(*now);
        tm local{};
        if (localtime_r(&value, &local)) std::strftime(clock, sizeof clock, "%H:%M", &local);
    }
    auto time = text(bar, clock, &lv_font_montserrat_16, kText);
    lv_obj_align(time, LV_ALIGN_RIGHT_MID, gear ? -36 : -8, 0);
    // Green: broker and server up. Amber: broker up, server down. Red: no broker.
    auto dot = lv_obj_create(bar);
    lv_obj_remove_style_all(dot);
    lv_obj_clear_flag(dot, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(dot, 8, 8);
    lv_obj_set_style_radius(dot, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_opa(dot, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(dot, lv_color_hex(!model_.mqtt_connected ? kBad : model_.server_online ? kGood : kWarn), 0);
    lv_obj_update_layout(time);
    lv_obj_align_to(dot, time, LV_ALIGN_OUT_LEFT_MID, -6, 0);
}

void Ui::overview(std::optional<int64_t> now) {
    switch (model_.config.layout) {
    case Layout::Cards: cards_overview(now); break;
    case Layout::Gauges: gauges_overview(now); break;
    case Layout::Terminal: terminal_overview(now); break;
    }
}

void Ui::cards_overview(std::optional<int64_t> now) {
    lv_obj_set_flex_flow(body_, LV_FLEX_FLOW_ROW);
    lv_obj_clear_flag(body_, LV_OBJ_FLAG_SCROLLABLE);
    provider_card(body_, Provider::Claude, now);
    provider_card(body_, Provider::Codex, now);
}

void Ui::gauges_overview(std::optional<int64_t> now) {
    lv_obj_set_flex_flow(body_, LV_FLEX_FLOW_ROW);
    lv_obj_clear_flag(body_, LV_OBJ_FLAG_SCROLLABLE);
    gauge_card(body_, Provider::Claude, now);
    gauge_card(body_, Provider::Codex, now);
}

void Ui::terminal_overview(std::optional<int64_t> now) {
    lv_obj_set_flex_flow(body_, LV_FLEX_FLOW_COLUMN);
    lv_obj_clear_flag(body_, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_pad_row(body_, 10, 0);
    mono(body_, colored(kMuted, "$ quota status"));
    terminal_block(body_, Provider::Claude, now);
    terminal_block(body_, Provider::Codex, now);
}

void Ui::gauge_card(lv_obj_t* parent, Provider provider, std::optional<int64_t> now) {
    auto panel = tap_target(card(parent), provider == Provider::Claude ? ClaudeDetail : CodexDetail);
    lv_obj_set_height(panel, LV_PCT(100));
    lv_obj_set_flex_grow(panel, 1);
    lv_obj_set_style_pad_row(panel, 2, 0);
    lv_obj_set_flex_align(panel, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);

    const auto snapshot = model_.snapshot(provider);
    card_header(panel, provider, status(snapshot, model_, now));
    const auto picked = snapshot ? overview_windows(*snapshot) : std::array<const Window*, 2>{};
    if (!picked[0]) {
        text(panel, no_window_message(snapshot), &lv_font_montserrat_14, kMuted, true);
        return;
    }
    // 270-degree ring, gap at the bottom. The ring shows what is left.
    const auto& main = *picked[0];
    auto ring = lv_arc_create(panel);
    lv_obj_clear_flag(ring, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_add_flag(ring, LV_OBJ_FLAG_EVENT_BUBBLE);
    lv_obj_set_size(ring, 96, 96);
    lv_arc_set_rotation(ring, 135);
    lv_arc_set_bg_angles(ring, 0, 270);
    lv_arc_set_range(ring, 0, 100);
    lv_arc_set_value(ring, main.remaining_percent ? static_cast<int16_t>(*main.remaining_percent + 0.5) : 0);
    lv_obj_remove_style(ring, nullptr, LV_PART_KNOB);
    lv_obj_set_style_arc_width(ring, 10, LV_PART_MAIN);
    lv_obj_set_style_arc_width(ring, 10, LV_PART_INDICATOR);
    lv_obj_set_style_arc_color(ring, lv_color_hex(kTrack), LV_PART_MAIN);
    lv_obj_set_style_arc_color(ring, lv_color_hex(accent(provider)), LV_PART_INDICATOR);
    lv_obj_center(text(ring, percent_left(main), &lv_font_montserrat_28, health(main)));
    text(panel, std::string(main.label.data()) + " left", &lv_font_montserrat_12, kMuted);
    text(panel, reset_line(main, now, false), &lv_font_montserrat_12, kMuted);
    if (const auto* weekly = picked[1]) {
        auto line = box(panel, LV_FLEX_FLOW_ROW, 6);
        lv_obj_set_style_pad_top(line, 3, 0);
        lv_obj_set_flex_align(line, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_END);
        text(line, percent_left(*weekly), &lv_font_montserrat_16, health(*weekly));
        text(line, std::string(weekly->label.data()) + " left", &lv_font_montserrat_12, kMuted);
        meter(panel, *weekly, accent(provider));
    }
}

void Ui::terminal_block(lv_obj_t* parent, Provider provider, std::optional<int64_t> now) {
    auto panel = tap_target(box(parent, LV_FLEX_FLOW_COLUMN), provider == Provider::Claude ? ClaudeDetail : CodexDetail);
    lv_obj_set_style_bg_color(panel, lv_color_hex(kCardPressed), LV_STATE_PRESSED);
    lv_obj_set_style_bg_opa(panel, LV_OPA_COVER, LV_STATE_PRESSED);
    lv_obj_set_style_radius(panel, 4, 0);
    lv_obj_set_style_pad_all(panel, 2, 0);
    lv_obj_set_style_pad_row(panel, 5, 0);
    const auto snapshot = model_.snapshot(provider);
    const auto state = status(snapshot, model_, now);
    std::string name = provider_name(provider);
    for (auto& c : name) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    char head[96];
    std::snprintf(head, sizeof head, "%-7s ", name.c_str());
    std::string line = colored(accent(provider), head) + colored(state.color, state.label);
    if (snapshot && snapshot->fetched_at) line += "  " + colored(kMuted, updated_line(*snapshot, now));
    mono(panel, line);
    const bool has_windows = snapshot && snapshot->window_count > 0;
    if (!has_windows) {
        mono(panel, colored(kMuted, "  " + no_window_message(snapshot)));
        return;
    }
    // Every reported window, labels padded to one column so values line up.
    std::vector<std::string> labels;
    size_t width = 0;
    for (size_t i = 0; i < snapshot->window_count && i < 6; ++i) {
        labels.push_back(short_label(snapshot->windows[i]));
        width = std::max(width, labels.back().size());
    }
    if (snapshot->banked_resets) width = std::max<size_t>(width, 6);
    for (size_t i = 0; i < labels.size(); ++i) {
        const auto& window = snapshot->windows[i];
        std::snprintf(head, sizeof head, "  %-*s ", static_cast<int>(width), labels[i].c_str());
        char percent[16];
        std::snprintf(percent, sizeof percent, "%4s", percent_left(window).c_str());
        mono(panel, head + colored(health(window), percent) + "  " + colored(kMuted, compact_reset(window, now)));
    }
    if (snapshot->banked_resets) {
        const auto& bank = *snapshot->banked_resets;
        std::snprintf(head, sizeof head, "  %-*s ", static_cast<int>(width), "banked");
        mono(panel, head + colored(kText, bank.available_count ? std::to_string(*bank.available_count) : "?"));
    }
}

void Ui::provider_card(lv_obj_t* parent, Provider provider, std::optional<int64_t> now) {
    auto panel = tap_target(card(parent), provider == Provider::Claude ? ClaudeDetail : CodexDetail);
    lv_obj_set_height(panel, LV_PCT(100));
    lv_obj_set_flex_grow(panel, 1);
    lv_obj_set_style_pad_row(panel, 2, 0);

    const auto snapshot = model_.snapshot(provider);
    card_header(panel, provider, status(snapshot, model_, now));

    const auto picked = snapshot ? overview_windows(*snapshot) : std::array<const Window*, 2>{};
    if (!picked[0]) text(panel, no_window_message(snapshot), &lv_font_montserrat_14, kMuted, true);
    for (const auto* window : picked) {
        if (!window) continue;
        const auto color = health(*window);
        auto line = box(panel, LV_FLEX_FLOW_ROW, 6);
        lv_obj_set_style_pad_top(line, 4, 0);
        lv_obj_set_flex_align(line, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_END);
        text(line, percent_left(*window), &lv_font_montserrat_28, color);
        auto caption = text(line, std::string(window->label.data()) + " left", &lv_font_montserrat_12, kMuted);
        lv_obj_set_style_pad_bottom(caption, 5, 0);
        meter(panel, *window, color);
        text(panel, reset_line(*window, now, false), &lv_font_montserrat_12, kMuted);
    }
    if (snapshot && snapshot->banked_resets) {
        const auto& bank = *snapshot->banked_resets;
        auto filler = box(panel, LV_FLEX_FLOW_ROW);
        lv_obj_set_flex_grow(filler, 1);
        std::string line = LV_SYMBOL_REFRESH " ";
        line += bank.available_count ? std::to_string(*bank.available_count) : "?";
        line += bank.available_count && *bank.available_count == 1 ? " banked reset" : " banked resets";
        text(panel, line, &lv_font_montserrat_12, kText);
    }
}

void Ui::detail(Provider provider, std::optional<int64_t> now) {
    lv_obj_set_flex_flow(body_, LV_FLEX_FLOW_COLUMN);
    const auto snapshot = model_.snapshot(provider);
    const auto state = status(snapshot, model_, now);

    auto summary = box(body_, LV_FLEX_FLOW_ROW, 8);
    lv_obj_set_flex_align(summary, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    pill(summary, state.label, state.color);
    text(summary, snapshot ? updated_line(*snapshot, now) : "No data yet", &lv_font_montserrat_12, kMuted);
    if (!snapshot) {
        text(body_, "Waiting for the server. Check the connection dot in the top right.", &lv_font_montserrat_14, kMuted, true);
        return;
    }
    if (const char* message = problem(*snapshot)) text(body_, message, &lv_font_montserrat_14, state.color == kBad ? kBad : kWarn, true);
    if (snapshot->mock) text(body_, "Demo data, not from your account.", &lv_font_montserrat_12, kWarn, true);

    for (size_t i = 0; i < snapshot->window_count; ++i) {
        const auto& window = snapshot->windows[i];
        const auto color = health(window);
        auto panel = card(body_);
        auto line = box(panel, LV_FLEX_FLOW_ROW);
        lv_obj_set_flex_align(line, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_END);
        text(line, window.label.data(), &lv_font_montserrat_16, kText);
        text(line, percent_left(window) + " left", &lv_font_montserrat_20, color);
        meter(panel, window, color);
        text(panel, reset_line(window, now, true), &lv_font_montserrat_12, kMuted);
    }

    if (!snapshot->banked_resets) return;
    const auto& bank = *snapshot->banked_resets;
    auto panel = card(body_);
    auto line = box(panel, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(line, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_END);
    text(line, "Banked resets", &lv_font_montserrat_16, kText);
    text(line, bank.available_count ? std::to_string(*bank.available_count) : "?", &lv_font_montserrat_20, accent(provider));
    for (size_t i = 0; i < bank.expiration_count; ++i) {
        const auto expiry = bank.expirations[i];
        const auto left = countdown(expiry, now);
        std::string row = !left ? "Expires " + local_time(expiry, std::nullopt)
            : *left == 0 ? "Expired, waiting for refresh"
            : "Expires in " + short_duration(*left) + " (" + local_time(expiry, std::nullopt) + ")";
        text(panel, row, &lv_font_montserrat_12, kMuted);
    }
    if (bank.known_nonexpiring_count)
        text(panel, std::to_string(bank.known_nonexpiring_count) + " never expire", &lv_font_montserrat_12, kMuted);
    if (bank.details_status == Details::Partial) text(panel, "Some expiry dates weren't reported.", &lv_font_montserrat_12, kMuted, true);
    if (bank.details_status == Details::Unavailable) text(panel, "Expiry dates weren't reported.", &lv_font_montserrat_12, kMuted, true);
    text(panel, "Use them on the Codex website.", &lv_font_montserrat_12, kMuted, true);
}

void Ui::settings_screen() {
    lv_obj_set_flex_flow(body_, LV_FLEX_FLOW_COLUMN);
    if (!setup_password_.empty()) {
        auto panel = card(body_);
        if (!notice_.empty()) text(panel, notice_, &lv_font_montserrat_14, kWarn, true);
        text(panel, "Set up Wi-Fi", &lv_font_montserrat_20, kText);
        text(panel, "1. On your phone, join", &lv_font_montserrat_12, kMuted);
        text(panel, setup_ssid_, &lv_font_montserrat_16, kText);
        text(panel, "Password", &lv_font_montserrat_12, kMuted);
        // Split into two groups of four so it reads like a PIN.
        auto spaced = setup_password_.size() == 8 ? setup_password_.substr(0, 4) + " " + setup_password_.substr(4) : setup_password_;
        text(panel, spaced, &lv_font_montserrat_28, accent(Provider::Codex));
        text(panel, "2. Then open", &lv_font_montserrat_12, kMuted);
        text(panel, setup_address_, &lv_font_montserrat_16, kText);
    }
    auto panel = card(body_);
    if (!notice_.empty() && setup_password_.empty()) text(panel, notice_, &lv_font_montserrat_14, kWarn, true);
    size_t start = 0;
    while (start <= settings_text_.size()) {
        auto end = settings_text_.find('\n', start);
        if (end == std::string::npos) end = settings_text_.size();
        const auto entry = settings_text_.substr(start, end - start);
        start = end + 1;
        if (entry.empty()) continue;
        const auto colon = entry.find(": ");
        if (colon == std::string::npos) { text(panel, entry, &lv_font_montserrat_12, kMuted, true); continue; }
        auto line = box(panel, LV_FLEX_FLOW_ROW);
        lv_obj_set_flex_align(line, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
        text(line, entry.substr(0, colon), &lv_font_montserrat_14, kMuted);
        text(line, entry.substr(colon + 2), &lv_font_montserrat_14, kText);
    }
    // While the setup AP is open the whole screen is already setup, so there is nothing to reconfigure.
    if (!setup_password_.empty()) return;
    // Last on the screen: a deliberate, two-step way to wipe Device setup.
    auto button = action_target(box(body_, LV_FLEX_FLOW_ROW), OpenConfirm);
    lv_obj_set_height(button, 36);
    lv_obj_set_flex_align(button, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_bg_opa(button, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(button, lv_color_hex(kTrack), 0);
    lv_obj_set_style_bg_color(button, lv_color_hex(kCardPressed), LV_STATE_PRESSED);
    lv_obj_set_style_radius(button, 8, 0);
    text(button, LV_SYMBOL_WIFI "  Reconfigure Wi-Fi", &lv_font_montserrat_14, kText);
}

void Ui::confirm_dialog() {
    // The scrim swallows taps so nothing behind the dialog can be pressed.
    auto scrim = lv_obj_create(root_);
    lv_obj_remove_style_all(scrim);
    lv_obj_clear_flag(scrim, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_add_flag(scrim, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_set_size(scrim, LV_PCT(100), LV_PCT(100));
    lv_obj_set_style_bg_opa(scrim, LV_OPA_80, 0);
    lv_obj_set_style_bg_color(scrim, lv_color_hex(0x000000), 0);
    auto panel = card(scrim);
    lv_obj_set_width(panel, 280);
    lv_obj_set_style_pad_all(panel, 10, 0);
    lv_obj_center(panel);
    text(panel, "Erase Wi-Fi and broker settings and start setup?", &lv_font_montserrat_16, kText, true);
    text(panel, "Layout, timezone and brightness are kept.", &lv_font_montserrat_12, kMuted, true);
    lv_obj_set_height(box(panel, LV_FLEX_FLOW_ROW), 3);
    // Cancel is the large, bright default.
    auto cancel = action_target(box(panel, LV_FLEX_FLOW_ROW), CloseConfirm);
    lv_obj_set_height(cancel, 40);
    lv_obj_set_flex_align(cancel, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_bg_opa(cancel, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(cancel, lv_color_hex(kText), 0);
    lv_obj_set_style_bg_color(cancel, lv_color_hex(kMuted), LV_STATE_PRESSED);
    lv_obj_set_style_radius(cancel, 8, 0);
    text(cancel, "Cancel", &lv_font_montserrat_16, kBackground);
    // Erase is small, outlined in the danger color and set apart at the far corner.
    lv_obj_set_height(box(panel, LV_FLEX_FLOW_ROW), 15);
    auto row = box(panel, LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(row, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    auto erase = action_target(box(row, LV_FLEX_FLOW_ROW), Erase);
    lv_obj_set_size(erase, 84, 30);
    lv_obj_set_flex_align(erase, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_border_width(erase, 1, 0);
    lv_obj_set_style_border_color(erase, lv_color_hex(kBad), 0);
    lv_obj_set_style_bg_opa(erase, LV_OPA_COVER, LV_STATE_PRESSED);
    lv_obj_set_style_bg_color(erase, lv_color_hex(kCardPressed), LV_STATE_PRESSED);
    lv_obj_set_style_radius(erase, 8, 0);
    text(erase, "Erase", &lv_font_montserrat_14, kBad);
}
}
