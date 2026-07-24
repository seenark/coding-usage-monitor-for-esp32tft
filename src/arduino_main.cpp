#include <Arduino.h>
#include <Preferences.h>
#include <TFT_eSPI.h>
#include <XPT2046_Touchscreen.h>
#include "usage_domain.h"
#include "wifi_manager.h"
#include "wifi_config_portal.h"

namespace board {
constexpr int LCD_SCLK = 14;
constexpr int LCD_MOSI = 13;
constexpr int LCD_MISO = 12;
constexpr int LCD_CS = 15;
constexpr int LCD_DC = 2;
constexpr int LCD_BACKLIGHT = 21;
constexpr int TOUCH_SCLK = 25;
constexpr int TOUCH_MOSI = 32;
constexpr int TOUCH_MISO = 39;
constexpr int TOUCH_CS_PIN = 33;
constexpr int TOUCH_IRQ = 36;
}

TFT_eSPI display;
SPIClass touchBus(HSPI);
XPT2046_Touchscreen touch(board::TOUCH_CS_PIN, board::TOUCH_IRQ);


Preferences preferences;
UsageState usage;
WifiManager wifi_manager;
WifiConfigPortal wifi_portal;
WifiState last_wifi_state = WifiState::Connecting;
MockUsageProvider usage_provider;
uint8_t current_screen = 0;

constexpr bool ENABLE_WIFI_RUNTIME = false;

bool touch_active = false;
int16_t touch_start_x = 0;

void draw_line(const String &text, int y) {
  display.setCursor(12, y);
  display.print(text);
}

void render_screen() {
  display.fillScreen(TFT_BLUE);
  display.setTextFont(1);
  display.setTextColor(TFT_WHITE, TFT_BLUE);
  display.setTextSize(2);
  if (current_screen == 0) {
    draw_line("Codex Usage", 16);
    draw_line("5-hour: " + String(usage.five_hour_used) + "%", 64);
    draw_line("Weekly: " + String(usage.weekly_used) + "%", 96);
    draw_line(usage.connectivity == Connectivity::Online ? "Online" : "Offline", 128);
  } else if (current_screen == 1) {
    draw_line("Gauges", 16);
    draw_line("5H " + String(usage.five_hour_used) + "%", 70);
    draw_line("WEEK " + String(usage.weekly_used) + "%", 110);
  } else {
    draw_line("Terminal", 16);
    draw_line("PLAN " + usage.plan, 70);
    draw_line("SYNC " + String((millis() - usage.last_updated_ms) / 1000) + " SEC AGO", 110);
  }
}

void probe_touch_bus() {
  uint8_t response[12] = {};
  const uint8_t commands[] = {0xB1, 0xC1, 0x91, 0xD1, 0x91, 0xD1};
  touchBus.beginTransaction(SPISettings(2000000, MSBFIRST, SPI_MODE0));
  digitalWrite(board::TOUCH_CS_PIN, LOW);
  for (size_t index = 0; index < 6; ++index) {
    response[index * 2] = touchBus.transfer(commands[index]);
    response[index * 2 + 1] = touchBus.transfer(0);
  }
  digitalWrite(board::TOUCH_CS_PIN, HIGH);
  touchBus.endTransaction();
  Serial.printf("touch probe irq=%d bytes=", digitalRead(board::TOUCH_IRQ));
  for (const uint8_t value : response) {
    Serial.printf("%02x", value);
  }
  Serial.println();
}
void setup() {
  preferences.begin("monitor", false);
  current_screen = preferences.getUChar("screen", 0) % 3;
  Serial.begin(115200);
  delay(250);
  pinMode(board::LCD_BACKLIGHT, OUTPUT);
  digitalWrite(board::LCD_BACKLIGHT, HIGH);
  display.init();
  display.setRotation(0);
  const uint16_t colors[] = {TFT_RED, TFT_GREEN, TFT_BLUE, TFT_WHITE, TFT_BLACK, TFT_YELLOW};
  const int band_height = display.height() / 6;
  for (int band = 0; band < 6; ++band) {
    display.fillRect(0, band * band_height, display.width(), band_height, colors[band]);
  }
  usage_provider.refresh(usage);
  render_screen();
  if (ENABLE_WIFI_RUNTIME) {
    // Start networking only after the TFT has been initialized.
    wifi_portal.begin();
    if (wifi_portal.has_credentials()) {
      wifi_manager.set_ssid(wifi_portal.ssid().c_str());
      wifi_manager.set_password(wifi_portal.password().c_str());
    }
    wifi_manager.begin();
  }
  pinMode(board::TOUCH_CS_PIN, OUTPUT);
  digitalWrite(board::TOUCH_CS_PIN, HIGH);
  pinMode(board::TOUCH_IRQ, INPUT);
  touch.begin(touchBus);
  touchBus.begin(board::TOUCH_SCLK, board::TOUCH_MISO, board::TOUCH_MOSI, board::TOUCH_CS_PIN);
  render_screen();
  Serial.println("ESP32-2432S028 candidate GPIO map loaded");
}

void loop() {
  if (ENABLE_WIFI_RUNTIME) {
    wifi_portal.tick();
    if (wifi_portal.has_credentials()) {
      wifi_manager.set_ssid(wifi_portal.ssid().c_str());
      wifi_manager.set_password(wifi_portal.password().c_str());
      wifi_manager.connect();
    }
    wifi_manager.tick();
    const WifiState wifi_state = wifi_manager.state();
    if (wifi_state != last_wifi_state) {
      last_wifi_state = wifi_state;
      usage.connectivity =
          wifi_state == WifiState::Online ? Connectivity::Online :
          wifi_state == WifiState::Connecting ? Connectivity::Connecting :
          Connectivity::Offline;
      render_screen();
      Serial.printf("wifi state=%u attempts=%lu\n",
                    static_cast<unsigned>(wifi_state),
                    static_cast<unsigned long>(wifi_manager.status().attempt_count));
    }
  }
  if (Serial.available()) {
    const char command = static_cast<char>(Serial.read());
    if (command >= '0' && command <= '2') {
      current_screen = static_cast<uint8_t>(command - '0');
      render_screen();
      preferences.putUChar("screen", current_screen);
    } else if (command == 'o') {
      usage.connectivity = Connectivity::Offline;
      render_screen();
      Serial.println("mock connectivity=offline");
    } else if (command == 'm') {
      usage.connectivity = Connectivity::Online;
      usage.last_updated_ms = millis();
      render_screen();
      Serial.println("mock connectivity=online");
    } else if (command == 'r') {
      probe_touch_bus();
    }
  }
  const bool irq_low = digitalRead(board::TOUCH_IRQ) == LOW;
  if (irq_low && touch.touched()) {
    const TS_Point point = touch.getPoint();
    if (!touch_active) {
      touch_active = true;
      touch_start_x = point.x;
    }
    Serial.printf("touch irq=0 raw=(%d,%d,%d) screen=%u\n",
                  point.x, point.y, point.z, current_screen);
  } else {
    if (touch_active) {
      const TS_Point point = touch.getPoint();
      const int16_t delta_x = point.x - touch_start_x;
      if (abs(delta_x) >= 200) {
        if (delta_x < 0) {
          current_screen = static_cast<uint8_t>((current_screen + 1) % 3);
        } else {
          current_screen = static_cast<uint8_t>((current_screen + 2) % 3);
        }
        preferences.putUChar("screen", current_screen);
        render_screen();
      }
      touch_active = false;
    }
    Serial.printf("touch idle irq=%d screen=%u\n",
                  digitalRead(board::TOUCH_IRQ), current_screen);
  }
  delay(250);
}
