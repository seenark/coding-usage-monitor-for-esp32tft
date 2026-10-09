#pragma once

// Experimental ILI9341 / XPT2046 preset, not a claim of physical validation.
// Initializes real SPI buses and registers an LVGL display and pointer device.
void board_init(int brightness_percent);
// Backlight PWM duty, 0..=100. No-op when the backlight GPIO is disabled.
void board_set_brightness(int percent);
