#include "board.hpp"

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <initializer_list>

#include "driver/gpio.h"
#include "driver/ledc.h"
#include "driver/spi_master.h"
#include "esp_attr.h"
#include "esp_check.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lvgl.h"
#include "sdkconfig.h"

namespace {
constexpr int kWidth = CONFIG_QUOTA_LCD_WIDTH;
constexpr int kHeight = CONFIG_QUOTA_LCD_HEIGHT;
constexpr int kRows = 16;
spi_device_handle_t lcd = nullptr;
spi_device_handle_t touch = nullptr;
#if CONFIG_QUOTA_LCD_BL_ACTIVE_HIGH
[[maybe_unused]] constexpr bool kBacklightActiveHigh = true;
#else
[[maybe_unused]] constexpr bool kBacklightActiveHigh = false;
#endif
DMA_ATTR lv_color_t pixels[kWidth * kRows];
lv_disp_draw_buf_t draw_buffer;
lv_disp_drv_t display_driver;
lv_indev_drv_t touch_driver;

void lcd_transfer(bool data, const void* bytes, size_t size) {
    if (!size) return;
    ESP_ERROR_CHECK(gpio_set_level(static_cast<gpio_num_t>(CONFIG_QUOTA_LCD_DC), data));
    spi_transaction_t transaction{};
    transaction.length = size * 8;
    if (size <= sizeof(transaction.tx_data)) {
        transaction.flags = SPI_TRANS_USE_TXDATA;
        std::memcpy(transaction.tx_data, bytes, size);
    } else {
        transaction.tx_buffer = bytes;
    }
    ESP_ERROR_CHECK(spi_device_polling_transmit(lcd, &transaction));
}

void command(uint8_t value, std::initializer_list<uint8_t> data = {}) {
    lcd_transfer(false, &value, 1);
    lcd_transfer(true, data.begin(), data.size());
}

void flush(lv_disp_drv_t* driver, const lv_area_t* area, lv_color_t* colors) {
    const uint16_t x1 = area->x1, x2 = area->x2, y1 = area->y1, y2 = area->y2;
    command(0x2a, {static_cast<uint8_t>(x1 >> 8), static_cast<uint8_t>(x1),
                   static_cast<uint8_t>(x2 >> 8), static_cast<uint8_t>(x2)});
    command(0x2b, {static_cast<uint8_t>(y1 >> 8), static_cast<uint8_t>(y1),
                   static_cast<uint8_t>(y2 >> 8), static_cast<uint8_t>(y2)});
    command(0x2c);
    const size_t count = static_cast<size_t>(x2 - x1 + 1) * (y2 - y1 + 1);
#if !LV_COLOR_16_SWAP
    // ILI9341 expects RGB565 most-significant byte first. Reuse LVGL's DMA buffer.
    for (size_t i = 0; i < count; ++i) colors[i].full = __builtin_bswap16(colors[i].full);
#endif
    lcd_transfer(true, colors, count * sizeof(lv_color_t));
    lv_disp_flush_ready(driver);
}

uint16_t touch_channel(uint8_t channel) {
    spi_transaction_t transaction{};
    transaction.flags = SPI_TRANS_USE_TXDATA | SPI_TRANS_USE_RXDATA;
    transaction.length = 24;
    transaction.tx_data[0] = channel;
    ESP_ERROR_CHECK(spi_device_polling_transmit(touch, &transaction));
    return (((static_cast<uint16_t>(transaction.rx_data[1]) << 8) | transaction.rx_data[2]) >> 3) & 0x0fff;
}

uint16_t median(uint8_t channel) {
    uint16_t a = touch_channel(channel), b = touch_channel(channel), c = touch_channel(channel);
    if (a > b) std::swap(a, b);
    if (b > c) std::swap(b, c);
    return std::max(a, b);
}

int map_coordinate(int value, int low, int high, int extent, bool invert) {
    const int position = std::clamp((value - low) * (extent - 1) / (high - low), 0, extent - 1);
    return invert ? extent - 1 - position : position;
}

void read_touch(lv_indev_drv_t*, lv_indev_data_t* data) {
    static lv_point_t last{};
    data->point = last;
    data->state = LV_INDEV_STATE_RELEASED;
#if CONFIG_QUOTA_TOUCH_IRQ >= 0
    if (gpio_get_level(static_cast<gpio_num_t>(CONFIG_QUOTA_TOUCH_IRQ))) return;
#endif
    // PD0=0 restores the XPT2046 PENIRQ after each conversion.
    if (touch_channel(0xb0) < CONFIG_QUOTA_TOUCH_PRESSURE_MIN) return;
    const int raw_x = median(0xd0), raw_y = median(0x90);
    const int x = map_coordinate(raw_x, CONFIG_QUOTA_TOUCH_X_MIN, CONFIG_QUOTA_TOUCH_X_MAX,
#if CONFIG_QUOTA_TOUCH_SWAP_XY
                                 kHeight,
#else
                                 kWidth,
#endif
#if CONFIG_QUOTA_TOUCH_INVERT_X
                                 true);
#else
                                 false);
#endif
    const int y = map_coordinate(raw_y, CONFIG_QUOTA_TOUCH_Y_MIN, CONFIG_QUOTA_TOUCH_Y_MAX,
#if CONFIG_QUOTA_TOUCH_SWAP_XY
                                 kWidth,
#else
                                 kHeight,
#endif
#if CONFIG_QUOTA_TOUCH_INVERT_Y
                                 true);
#else
                                 false);
#endif
#if CONFIG_QUOTA_TOUCH_SWAP_XY
    last.x = y;
    last.y = x;
#else
    last.x = x;
    last.y = y;
#endif
    data->point = last;
    data->state = LV_INDEV_STATE_PRESSED;
}

void output_pin(int pin) {
    gpio_config_t configuration{};
    configuration.pin_bit_mask = 1ULL << pin;
    configuration.mode = GPIO_MODE_OUTPUT;
    ESP_ERROR_CHECK(gpio_config(&configuration));
}
} // namespace

void board_set_brightness(int percent) {
#if CONFIG_QUOTA_LCD_BL >= 0
    constexpr uint32_t kMaxDuty = (1U << 10) - 1;
    const uint32_t duty = static_cast<uint32_t>(std::clamp(percent, 0, 100)) * kMaxDuty / 100;
    ESP_ERROR_CHECK(ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0, duty));
    ESP_ERROR_CHECK(ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0));
#else
    (void)percent;
#endif
}

void board_init(int brightness_percent) {
    static_assert(LV_COLOR_DEPTH == 16, "The ILI9341 driver requires LVGL RGB565");
    static_assert(CONFIG_QUOTA_TOUCH_X_MAX > CONFIG_QUOTA_TOUCH_X_MIN, "Invalid touch X calibration");
    static_assert(CONFIG_QUOTA_TOUCH_Y_MAX > CONFIG_QUOTA_TOUCH_Y_MIN, "Invalid touch Y calibration");
    output_pin(CONFIG_QUOTA_LCD_DC);
#if CONFIG_QUOTA_LCD_BL >= 0
    // Backlight stays dark until the first brightness is applied after init.
    ledc_timer_config_t backlight_timer{};
    backlight_timer.speed_mode = LEDC_LOW_SPEED_MODE;
    backlight_timer.duty_resolution = LEDC_TIMER_10_BIT;
    backlight_timer.timer_num = LEDC_TIMER_0;
    backlight_timer.freq_hz = 5000;
    backlight_timer.clk_cfg = LEDC_AUTO_CLK;
    ESP_ERROR_CHECK(ledc_timer_config(&backlight_timer));
    ledc_channel_config_t backlight{};
    backlight.gpio_num = CONFIG_QUOTA_LCD_BL;
    backlight.speed_mode = LEDC_LOW_SPEED_MODE;
    backlight.channel = LEDC_CHANNEL_0;
    backlight.timer_sel = LEDC_TIMER_0;
    backlight.duty = 0;
    backlight.flags.output_invert = !kBacklightActiveHigh;
    ESP_ERROR_CHECK(ledc_channel_config(&backlight));
#endif
#if CONFIG_QUOTA_LCD_RST >= 0
    output_pin(CONFIG_QUOTA_LCD_RST);
    ESP_ERROR_CHECK(gpio_set_level(static_cast<gpio_num_t>(CONFIG_QUOTA_LCD_RST), 0));
    vTaskDelay(pdMS_TO_TICKS(20));
    ESP_ERROR_CHECK(gpio_set_level(static_cast<gpio_num_t>(CONFIG_QUOTA_LCD_RST), 1));
    vTaskDelay(pdMS_TO_TICKS(120));
#endif
    spi_bus_config_t bus{};
    bus.mosi_io_num = CONFIG_QUOTA_LCD_MOSI;
    bus.miso_io_num = CONFIG_QUOTA_LCD_MISO;
    bus.sclk_io_num = CONFIG_QUOTA_LCD_SCLK;
    bus.quadwp_io_num = -1;
    bus.quadhd_io_num = -1;
    bus.max_transfer_sz = sizeof(pixels);
    ESP_ERROR_CHECK(spi_bus_initialize(SPI2_HOST, &bus, SPI_DMA_CH_AUTO));
    spi_device_interface_config_t device{};
    device.clock_speed_hz = CONFIG_QUOTA_LCD_SPI_HZ;
    device.spics_io_num = CONFIG_QUOTA_LCD_CS;
    device.queue_size = 1;
    ESP_ERROR_CHECK(spi_bus_add_device(SPI2_HOST, &device, &lcd));

    // Standard ILI9341 power, gamma and RGB565 initialization. Controller identity
    // is unresolved on this board; these commands are an experimental preset.
    command(0x01);
    vTaskDelay(pdMS_TO_TICKS(150));
    command(0x28);
    command(0xef, {0x03, 0x80, 0x02});
    command(0xcf, {0x00, 0xc1, 0x30});
    command(0xed, {0x64, 0x03, 0x12, 0x81});
    command(0xe8, {0x85, 0x00, 0x78});
    command(0xcb, {0x39, 0x2c, 0x00, 0x34, 0x02});
    command(0xf7, {0x20});
    command(0xea, {0x00, 0x00});
    command(0xc0, {0x23});
    command(0xc1, {0x10});
    command(0xc5, {0x3e, 0x28});
    command(0xc7, {0x86});
    command(0x36, {CONFIG_QUOTA_LCD_MADCTL});
    command(0x3a, {0x55});
    command(0xb1, {0x00, 0x18});
    command(0xb6, {0x08, 0x82, 0x27});
    command(0xf2, {0x00});
    command(0x26, {0x01});
    command(0xe0, {0x0f, 0x31, 0x2b, 0x0c, 0x0e, 0x08, 0x4e, 0xf1, 0x37, 0x07, 0x10, 0x03, 0x0e, 0x09, 0x00});
    command(0xe1, {0x00, 0x0e, 0x14, 0x03, 0x11, 0x07, 0x31, 0xc1, 0x48, 0x08, 0x0f, 0x0c, 0x31, 0x36, 0x0f});
#if CONFIG_QUOTA_LCD_INVERT
    command(0x21);
#else
    command(0x20);
#endif
    command(0x11);
    vTaskDelay(pdMS_TO_TICKS(120));
    command(0x29);
    vTaskDelay(pdMS_TO_TICKS(20));

    spi_bus_config_t touch_bus{};
    touch_bus.mosi_io_num = CONFIG_QUOTA_TOUCH_MOSI;
    touch_bus.miso_io_num = CONFIG_QUOTA_TOUCH_MISO;
    touch_bus.sclk_io_num = CONFIG_QUOTA_TOUCH_SCLK;
    touch_bus.quadwp_io_num = -1;
    touch_bus.quadhd_io_num = -1;
    touch_bus.max_transfer_sz = 4;
    ESP_ERROR_CHECK(spi_bus_initialize(SPI3_HOST, &touch_bus, SPI_DMA_DISABLED));
    spi_device_interface_config_t touch_device{};
    touch_device.clock_speed_hz = CONFIG_QUOTA_TOUCH_SPI_HZ;
    touch_device.spics_io_num = CONFIG_QUOTA_TOUCH_CS;
    touch_device.queue_size = 1;
    ESP_ERROR_CHECK(spi_bus_add_device(SPI3_HOST, &touch_device, &touch));
#if CONFIG_QUOTA_TOUCH_IRQ >= 0
    gpio_config_t interrupt_pin{};
    interrupt_pin.pin_bit_mask = 1ULL << CONFIG_QUOTA_TOUCH_IRQ;
    interrupt_pin.mode = GPIO_MODE_INPUT;
    // GPIO36 has no internal pull-up. The module must provide its own pull-up.
    ESP_ERROR_CHECK(gpio_config(&interrupt_pin));
#endif
    lv_disp_draw_buf_init(&draw_buffer, pixels, nullptr, kWidth * kRows);
    lv_disp_drv_init(&display_driver);
    display_driver.hor_res = kWidth;
    display_driver.ver_res = kHeight;
    display_driver.draw_buf = &draw_buffer;
    display_driver.flush_cb = flush;
    if (!lv_disp_drv_register(&display_driver)) ESP_ERROR_CHECK(ESP_ERR_NO_MEM);
    lv_indev_drv_init(&touch_driver);
    touch_driver.type = LV_INDEV_TYPE_POINTER;
    touch_driver.read_cb = read_touch;
    if (!lv_indev_drv_register(&touch_driver)) ESP_ERROR_CHECK(ESP_ERR_NO_MEM);
    board_set_brightness(brightness_percent);
}
