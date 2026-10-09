# Hardware inspection report

## Confirmed on attached board

- **Board identity:** The prior probe record identifies an ESP32-D0WD-V3, revision v3.1, with a 40 MHz crystal, 4 MB flash, and MAC `20:50:0d:1a:a2:0c`. Source: [`hardware-report.txt`](../hardware-report.txt), lines 6–20.
- **Physical family:** The supplied rear-board observation records `ESP32-2432S028`, an Espressif ESP-WROOM-32 module, micro-USB connector, CH340 bridge, BOOT/RST buttons, and micro-SD socket. Source: [`hardware-report.txt`](../hardware-report.txt), lines 22–28.
- **USB bridge:** CH340 VID:PID `0x1a86:0x7523`. Source: [`hardware-report.txt`](../hardware-report.txt), lines 22–25.
- **Serial transport:** `/dev/cu.usbserial-83340` exists. The retained `pio device monitor -p /dev/cu.usbserial-83340 -b 115200` process holds the port; `lsof /dev/cu.usbserial-83340` reported the active Python monitor process (PID 86497). This inspection did not open, stop, reset, or otherwise interact with the serial stream.
- **Flash and security status:** The prior probe reports 4 MB flash and no secure boot, flash encryption, or NVS encryption. Source: [`hardware-report.txt`](../hardware-report.txt), lines 8–12 and 30–35.
- **Retained visual evidence:** The prior bring-up record says the ILI9341 candidate rendered six horizontal color bands. This is historical evidence from the retained record, not a new device probe. Source: [`hardware-report.txt`](../hardware-report.txt), lines 30–34.

## Documented v0.1 baseline

The micro-USB/CH340 evidence and `ESP32-2432S028` silkscreen classify this board as the documented v0.1 baseline. The LCD controller and active geometry are not fully identified by readable ID commands.

- **Display runtime evidence:** The ILI9342-compatible TFT_eSPI profile renders a clean four-quadrant **240×240 active square**. The remaining lower glass area remains inactive/backlight-only. ILI9341 rendering produced a different, smaller addressable region. This is the strongest current display evidence, but it does not prove the controller part number.
- **Display candidate pins:** SCLK GPIO14, MOSI GPIO13, MISO GPIO12, CS GPIO15, DC/RS GPIO2, reset tied to EN (`TFT_RST=-1`).
- **Backlight:** GPIO21, active high.
- **Touch candidate:** XPT2046 on an independent SPI bus.
  - CLK GPIO25
  - MOSI GPIO32
  - MISO GPIO39
  - CS GPIO33
  - IRQ GPIO36
- **Orientation:** The maintained 2432S028 examples document `tft.setRotation(1)` / LVGL `rotation: 90`, but the attached panel's active geometry is not yet mapped to that orientation.
- **Bus boundary:** Use the explicit 2432S028 separate-bus pattern. Do not substitute a generic CYD configuration that shares TFT and touch SPI pins.
- **Touch transform:** Raw corner samples are confirmed. The provisional mapping is raw X `512..3340` → screen Y and raw Y `408..3370` → screen X; final calibration remains pending.

Evidence:

- Maintained pin map: <https://github.com/witnessmenow/ESP32-Cheap-Yellow-Display/blob/main/PINS.md#L89-L125>
- Archived vendor v0.1 schematic: <https://github.com/witnessmenow/ESP32-Cheap-Yellow-Display/blob/main/OriginalDocumentation/5-Schematic/ESP32-2432S028-MCU.jpg>

## Unverified / blocked

- **PSRAM: Unverified.** ESP32-D0WD-V3 and ESP-WROOM-32 identification does not prove whether PSRAM is present or absent. This inspection performed no runtime PSRAM probe.
- **Touch operation: Confirmed baseline.** The flashed raw XPT2046 probe produced IRQ-low samples at all four corners, with stable clusters approximately `(x=512,y=360)`, `(512,3450)`, `(3338,3290)`, and `(3490,455)` in the requested top-left, top-right, bottom-right, bottom-left order. IRQ returned high when untouched. Calibration is not yet encoded in the application.
- **Current LVGL demo recovery:** Superseded by the user's explicit clean-start decision. No backup was retained; the cancelled readback created no file. The new LCD/touch diagnostic is the intentional replacement baseline.
- **Flash operations:** A clean LCD-only diagnostic, followed by the raw touch diagnostic, was flashed at the standard PlatformIO offsets. Each upload reported verified hashes. No credentials, security features, or destructive erase operation was used.
- **Next prerequisite:** Add calibrated touch mapping and the first offline MVP screen only after the user confirms the physical touch coordinates are acceptable.

## Evidence

### Host checks performed

- `stat /dev/cu.usbserial-*` → sole device `/dev/cu.usbserial-83340`.
- `lsof /dev/cu.usbserial-83340` → active Python PlatformIO monitor process holds the character device.
- `esptool image-info .pio/build/esp32dev/firmware.bin` → ESP32 image, 4 MB, DIO, 40 MHz, valid checksum and validation hash; image size 372448 bytes; validation hash `efc6149cc2bd454613592bacaa2dcbf916f925b0f6b8af71c8b0aac47a1e5d9c`.
- `esptool image-info .pio/build/displaydiag/firmware.bin` → ESP32 image, 4 MB, DIO, 40 MHz, valid checksum and validation hash; image size 284928 bytes; validation hash `f5fe3dacb63dd3d8f3677e4ac599c2bb8f0009b482e97f73907974b119411e92`.
- [`partitions.csv`](../partitions.csv) → bootloader-compatible layout with partition table at `0x8000`, factory app at `0x10000`, and 4 MB factory region through `0x3F0000`.
- [`flasher_args.json`](../target/xtensa-esp32-espidf/release/build/esp-idf-sys-c0f80e6c5809356c/out/build/flasher_args.json) → separate Rust artifact metadata: DIO, 4 MB, 40 MHz; bootloader `0x1000`, partition table `0x8000`, app `0x10000`. These offsets do not establish that artifact as the running demo.

### Inspection boundary

This report began as a non-destructive hardware inspection. The user then explicitly authorized a clean replacement path. The current repository contains the PlatformIO LCD/touch baseline plus a modular offline dashboard, status screen, and provider-neutral mock usage source (`platformio.ini`, `src/main.cpp`, `src/usage_state.h`, `src/usage_provider.h`, `src/mock_usage_provider.h`, `src/mock_usage_provider.cpp`, `src/dashboard.h`, `src/dashboard.cpp`, `src/status_screen.h`, and `src/status_screen.cpp`); no credentials or security provisioning were added. The ILI9341 landscape profile (`rotation=1`) is verified as `320×240`, with text and live mock values across the full panel. Touch IRQ/raw-corner behavior and debounced tap navigation between dashboard and status are confirmed. PSRAM remains unverified. Network provider integration, persisted production touch calibration, and final controller identification remain next.

The Wi-Fi settings screen and bounded reconnect manager are also verified in their credential-safe initial state: `NOT CONFIGURED` / `CREDENTIALS REQUIRED` / `OFFLINE MODE ACTIVE`. The three-screen tap cycle Dashboard → Status → Wi-Fi Settings → Dashboard completed on the attached board. Wi-Fi credentials remain absent from source, serial output, and NVS; no connection attempt is made until a future explicit provisioning flow supplies them in RAM.

### ESP-IDF firmware flashed (2026-10-08)

The board now runs the ESP-IDF/LVGL firmware from `firmware/`, replacing the Arduino baseline. It was built with `espressif/idf:v5.3.3` (app `0x118d00` bytes, 72% of the `0x3F0000` factory partition free) and written with `esptool` at the standard offsets. Every segment's hash verified. After reset, serial showed a normal ROM boot (`POWERON_RESET`, `SPI_FAST_FLASH_BOOT`) and no panic or reboot within 15 s. Application logging is deliberately disabled (`CONFIG_LOG_DEFAULT_LEVEL_NONE`), so serial cannot confirm display, touch, or setup-AP behavior. Those still require direct observation of the panel. To restore the Arduino baseline, rebuild and upload `src/` with the existing `platformio.ini`.

The user then observed the panel directly. The setup screen (`Temporary WPA2 setup`, SSID, 8-digit password, setup address) rendered legibly across the full display. This confirms the ILI9341 preset, 320×240 landscape geometry, and orientation on this board. Touch navigation, phone provisioning, and MQTT reception remain unobserved.

Two firmware fixes were then flashed. The setup HTTP guard now accepts IPv4-mapped peers, because ESP-IDF's `httpd` listens on an IPv6 socket when `CONFIG_LWIP_IPV6` is on. `CONFIG_HTTPD_MAX_REQ_HDR_LEN` was raised to 2048, because a phone browser's form POST exceeded the 512-byte default. After these fixes, the user provisioned the board from a phone over the setup AP. The board joined home Wi-Fi, switched from Settings to Overview, and showed live Codex quota from the LAN broker; the user reported the values as correct. Touch navigation between tabs, reboot behavior (RAM-only settings), and the Claude provider remain unobserved.

### Firmware: NVS, backlight PWM and display layouts (built, not flashed)

The firmware now stores Device setup and the last-known timezone, layout and brightness in NVS namespace `quota`, unencrypted (ADR 0002), so a reboot goes straight to Wi-Fi without the setup AP. The setup form no longer has a timezone field; timezone, layout and brightness arrive on `ai-quota/v1/device/config`. Brightness is a 5 kHz, 10-bit LEDC PWM on the backlight GPIO21 (still active high). The Settings screen ends with Reconfigure Wi-Fi, which erases the stored Wi-Fi and broker values after a confirmation and reopens the setup AP. This was verified only in the host build, the LVGL simulator and an ESP-IDF compile; the PWM backlight, NVS persistence, the reconfigure flow and the new layouts on the panel remain unobserved until flashed.
