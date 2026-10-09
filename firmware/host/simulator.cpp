#define SDL_MAIN_HANDLED
#include <SDL.h>
#include "quota_ui.hpp"
#include <algorithm>
#include <ctime>
#include <array>
#include <chrono>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <iterator>
#include <stdexcept>

namespace {
constexpr int width = 320, height = 240;
std::array<uint16_t,width * height> pixels{};
int mouse_x = 0, mouse_y = 0;
bool pressed = false;
void flush(lv_disp_drv_t* driver, const lv_area_t* area, lv_color_t* colors) {
    for (int y = area->y1; y <= area->y2; ++y)
        for (int x = area->x1; x <= area->x2; ++x) {
            const auto color = *colors++;
            if (x >= 0 && x < width && y >= 0 && y < height) pixels[y * width + x] = color.full;
        }
    lv_disp_flush_ready(driver);
}
void pointer(lv_indev_drv_t*, lv_indev_data_t* data) {
    data->point.x = static_cast<lv_coord_t>(mouse_x);
    data->point.y = static_cast<lv_coord_t>(mouse_y);
    data->state = pressed ? LV_INDEV_STATE_PR : LV_INDEV_STATE_REL;
}
std::string fixture(const std::string& path) {
    std::ifstream input(path,std::ios::binary);
    if (!input) throw std::runtime_error("fixture cannot be opened");
    std::string data;
    data.resize(quota::max_snapshot_bytes + 1);
    input.read(data.data(),static_cast<std::streamsize>(data.size()));
    data.resize(static_cast<size_t>(input.gcount()));
    if (data.size() > quota::max_snapshot_bytes) throw std::runtime_error("fixture too large");
    return data;
}
int64_t monotonic_ms() {
    return std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch()).count();
}
}
int main(int argc, char** argv) {
    std::string directory = "firmware/fixtures", scenario = "normal", screenshot, view = "overview", layout = "cards";
    int frames = 0;
    try {
        for (int i = 1; i < argc; ++i) {
            const std::string arg = argv[i];
            if (arg == "--help") {
                std::cout << "quota_simulator [--fixtures DIR] [--scenario normal|stale|ahead|behind|unknown|expired|offline|reauth|error|setup|setup_failed] [--layout cards|gauges|terminal] [--view overview|claude|codex|settings|confirm] [--frames N] [--screenshot FILE.bmp]\n";
                return 0;
            }
            if (i + 1 >= argc) throw std::runtime_error("option needs a value");
            const std::string value = argv[++i];
            if (arg == "--fixtures") directory = value;
            else if (arg == "--scenario") scenario = value;
            else if (arg == "--view") view = value;
            else if (arg == "--layout") layout = value;
            else if (arg == "--screenshot") screenshot = value;
            else if (arg == "--frames") { frames = std::stoi(value); if (frames < 1) throw std::runtime_error("frames must be positive"); }
            else throw std::runtime_error("unknown option");
        }
        const std::array<std::string,11> scenarios = {"normal","stale","ahead","behind","unknown","expired","offline","reauth","error","setup","setup_failed"};
        if (std::find(scenarios.begin(),scenarios.end(),scenario) == scenarios.end()) throw std::runtime_error("unknown scenario");
        const std::array<std::string,5> views = {"overview","claude","codex","settings","confirm"};
        const auto selected = std::find(views.begin(),views.end(),view);
        if (selected == views.end()) throw std::runtime_error("unknown view");
        const int selected_view = static_cast<int>(selected - views.begin());
        setenv("TZ","ICT-7",1); tzset();
        quota::Model model;
        model.mqtt_connected = scenario != "offline";
        model.server_online = scenario != "offline";
        const auto parsed_layout = quota::layout_from_name(layout);
        if (!parsed_layout) throw std::runtime_error("unknown layout");
        model.config.layout = *parsed_layout;
        for (const auto provider : {"claude","codex"}) {
            auto data = fixture(directory + "/" + provider + "-v1.json");
            if (scenario == "reauth" || scenario == "error") {
                const auto at = data.find("\"state\":\"ok\"");
                data.replace(at,12,scenario == "reauth" ? "\"state\":\"reauth_required\"" : "\"state\":\"error\"");
            }
            if (model.ingest(std::string("ai-quota/v1/providers/") + provider,data) != quota::ApplyResult::Applied) throw std::runtime_error("fixture validation failed");
        }
        quota::Clock clock;
        const auto start = monotonic_ms();
        int64_t now = 1791417660;
        if (scenario == "stale") now += 1800;
        if (scenario == "ahead") now += 86400;
        if (scenario == "behind") now -= 86400;
        if (scenario == "expired") now += 7200;
        if (scenario != "unknown") clock.sync(now,start);
        SDL_SetMainReady();
        if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_TIMER) != 0) throw std::runtime_error("SDL initialization failed");
        auto window = SDL_CreateWindow("AI Quota Monitor - DEMO synthetic fixtures",SDL_WINDOWPOS_CENTERED,SDL_WINDOWPOS_CENTERED,width * 2,height * 2,SDL_WINDOW_RESIZABLE);
        if (!window) throw std::runtime_error("SDL window creation failed");
        auto renderer = SDL_CreateRenderer(window,-1,SDL_RENDERER_SOFTWARE);
        if (!renderer) throw std::runtime_error("SDL renderer creation failed");
        SDL_RenderSetLogicalSize(renderer,width,height);
        auto texture = SDL_CreateTexture(renderer,SDL_PIXELFORMAT_RGB565,SDL_TEXTUREACCESS_STREAMING,width,height);
        if (!texture) throw std::runtime_error("SDL texture creation failed");
        lv_init();
        static std::array<lv_color_t,width * 20> buffer{};
        static lv_disp_draw_buf_t draw;
        lv_disp_draw_buf_init(&draw,buffer.data(),nullptr,buffer.size());
        static lv_disp_drv_t display;
        lv_disp_drv_init(&display); display.hor_res = width; display.ver_res = height;
        display.draw_buf = &draw; display.flush_cb = flush; lv_disp_drv_register(&display);
        static lv_indev_drv_t input;
        lv_indev_drv_init(&input); input.type = LV_INDEV_TYPE_POINTER; input.read_cb = pointer; lv_indev_drv_register(&input);
        quota::Ui ui(model,clock); ui.create();
        ui.settings("Wi-Fi: simulator\nMQTT: fixtures\nClock: synthetic\nTimezone: ICT-7\nDisplay: cards, 80%");
        if (scenario == "offline") ui.notice("Cannot join Wi-Fi. Open Settings > Reconfigure Wi-Fi.");
        // Synthetic setup values; the device generates a fresh password each boot.
        if (scenario == "setup" || scenario == "setup_failed") ui.setup("Usage-Setup-A1B2C3","12345678","http://192.168.4.1/");
        if (scenario == "setup_failed") ui.notice("Could not join Wi-Fi. Check the password and try again.");
        bool running = true;
        int rendered = 0;
        auto previous = start;
        while (running && (!frames || rendered < frames)) {
            SDL_Event event;
            while (SDL_PollEvent(&event)) {
                if (event.type == SDL_QUIT) running = false;
                if (event.type == SDL_KEYDOWN && event.key.keysym.sym == SDLK_ESCAPE) running = false;
                if (event.type == SDL_MOUSEMOTION || event.type == SDL_MOUSEBUTTONDOWN || event.type == SDL_MOUSEBUTTONUP) {
                    int x, y; SDL_GetMouseState(&x,&y);
                    float logical_x, logical_y;
                    SDL_RenderWindowToLogical(renderer,x,y,&logical_x,&logical_y);
                    mouse_x = static_cast<int>(logical_x); mouse_y = static_cast<int>(logical_y);
                    if (event.type == SDL_MOUSEBUTTONDOWN) pressed = true;
                    if (event.type == SDL_MOUSEBUTTONUP) pressed = false;
                }
            }
            // Exercise the same touch path as the device: tap a card or the gear.
            if (selected_view && rendered < 8) {
                const bool terminal = quota::layout_from_name(layout) == quota::Layout::Terminal;
                const int targets[5][2] = {{0,0},{terminal ? 160 : 80,terminal ? 70 : 130},{terminal ? 160 : 240,terminal ? 140 : 130},{304,15},{304,15}};
                mouse_x = targets[selected_view][0]; mouse_y = targets[selected_view][1]; pressed = rendered < 4;
            }
            // The confirm view then taps "Reconfigure Wi-Fi" on the Settings screen. It is never confirmed.
            if (selected_view == 4 && rendered >= 16 && rendered < 24) {
                mouse_x = 160; mouse_y = 198; pressed = rendered < 20;
            }
            const auto time = monotonic_ms();
            lv_tick_inc(static_cast<uint32_t>(time - previous)); previous = time;
            ui.tick(time); lv_timer_handler();
            SDL_UpdateTexture(texture,nullptr,pixels.data(),width * sizeof(uint16_t));
            SDL_RenderClear(renderer); SDL_RenderCopy(renderer,texture,nullptr,nullptr); SDL_RenderPresent(renderer);
            ++rendered; SDL_Delay(16);
        }
        if (!screenshot.empty()) {
            auto surface = SDL_CreateRGBSurfaceWithFormatFrom(pixels.data(),width,height,16,width * sizeof(uint16_t),SDL_PIXELFORMAT_RGB565);
            if (!surface || SDL_SaveBMP(surface,screenshot.c_str()) != 0) throw std::runtime_error("screenshot save failed");
            SDL_FreeSurface(surface);
        }
        SDL_DestroyTexture(texture); SDL_DestroyRenderer(renderer); SDL_DestroyWindow(window); SDL_Quit();
        return 0;
    } catch (const std::exception& error) {
        std::cerr << "Simulator: " << error.what() << '\n';
        SDL_Quit(); return 1;
    }
}
