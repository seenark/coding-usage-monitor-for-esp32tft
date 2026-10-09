#pragma once
#include "quota_model.hpp"
#include "lvgl.h"
#include <optional>
#include <string>
#include <utility>

namespace quota {
class Ui {
public:
    Ui(Model& model, Clock& clock) : model_(model), clock_(clock) {}
    void create();
    // Call every loop iteration. Redraws only when data, the screen, or the minute changes.
    void tick(int64_t monotonic_ms);
    // Shows the temporary setup Wi-Fi; pass nullptrs once setup is over.
    void setup(const char* ssid, const char* password, const char* address);
    // "Key: value" lines shown on the Settings screen.
    void settings(const char* text);
    // One plain sentence about connectivity ("Cannot join Wi-Fi..."); empty clears it.
    void notice(const char* text);
    // True once, after the user confirmed "Reconfigure Wi-Fi".
    bool take_reconfigure_request();

    enum Screen : unsigned { Overview, ClaudeDetail, CodexDetail, Settings };

private:
    enum Action : unsigned { OpenConfirm = 1, CloseConfirm, Erase };
    static void on_click(lv_event_t* event);
    static void on_action(lv_event_t* event);
    lv_obj_t* tap_target(lv_obj_t* object, Screen target);
    lv_obj_t* action_target(lv_obj_t* object, Action action);
    std::string state_key(std::optional<int64_t> now) const;
    std::string no_window_message(const Snapshot* snapshot) const;
    void render(std::optional<int64_t> now);
    void top_bar(std::optional<int64_t> now);
    void overview(std::optional<int64_t> now);
    void cards_overview(std::optional<int64_t> now);
    void gauges_overview(std::optional<int64_t> now);
    void terminal_overview(std::optional<int64_t> now);
    void provider_card(lv_obj_t* parent, Provider provider, std::optional<int64_t> now);
    void gauge_card(lv_obj_t* parent, Provider provider, std::optional<int64_t> now);
    void terminal_block(lv_obj_t* parent, Provider provider, std::optional<int64_t> now);
    void detail(Provider provider, std::optional<int64_t> now);
    void settings_screen();
    void confirm_dialog();

    Model& model_;
    Clock& clock_;
    Screen selected_ = Overview;
    Screen rendered_ = Overview;
    // Navigation is applied in tick(), never inside an LVGL event: rendering
    // deletes the object that received the click.
    std::optional<Screen> pending_;
    std::optional<Action> pending_action_;
    bool confirm_open_ = false;
    bool reconfigure_requested_ = false;
    lv_obj_t* root_ = nullptr;
    lv_obj_t* body_ = nullptr;
    int64_t last_tick_second_ = -1;
    std::string key_;
    std::string settings_text_ = "Wi-Fi: not configured";
    std::string notice_;
    std::string setup_ssid_, setup_password_, setup_address_;
};
}
