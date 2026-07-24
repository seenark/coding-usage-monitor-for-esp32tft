#include <Arduino.h>
#include <TFT_eSPI.h>

TFT_eSPI display;

void draw_line(const char *text, int y) {
  display.setCursor(12, y);
  display.print(text);
}

void setup() {
  Serial.begin(115200);
  delay(250);
  pinMode(21, OUTPUT);
  digitalWrite(21, HIGH);
  display.init();
  display.setRotation(0);
  display.fillScreen(TFT_BLUE);
  display.setTextFont(1);
  display.setTextSize(2);
  display.setTextColor(TFT_WHITE, TFT_BLUE);
  draw_line("Codex Usage", 16);
  draw_line("5-hour: 76%", 64);
  draw_line("Weekly: 48%", 96);
  draw_line("Offline", 128);
  Serial.println("clean dashboard: alive");
}

void loop() {
  delay(1000);
  Serial.println("clean dashboard: alive");
}
