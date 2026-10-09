# เฟิร์มแวร์ AI Quota Monitor

เฟิร์มแวร์นี้ใช้ ESP-IDF C++ และ LVGL 8.3.11 โดยแชร์ตัวถอดรหัส snapshot และหน้าจอจริงกับ simulator บนคอมพิวเตอร์ ไม่ใช้ Arduino และไม่มี token ของ Claude/Codex บนอุปกรณ์

## ส่วนที่ใช้ร่วมกัน

- `shared/quota_model.*`: ถอดรหัส cJSON, ตรวจ schema v1 และ provider ให้ตรง MQTT topic, จำกัด payload 16,384 bytes, windows 32 รายการ, expiry 64 รายการ และข้อความ 64 bytes ตรวจ duplicate JSON keys, UTF-8, nesting และตัวเลขก่อนนำไปใช้
- `shared/quota_ui.*`: ธีมมืด Overview มีการ์ด Claude และ Codex คู่กัน แสดง % ที่เหลือตัวใหญ่ (เขียว ≥50%, เหลือง ≥20%, แดง <20%), แถบ และ "Resets in 4h 12m" แตะการ์ดเพื่อเปิดหน้ารายละเอียดซึ่งเลื่อนดูได้ทุก window, เวลา reset และ banked resets ปุ่มเฟืองมุมขวาบนเปิด Settings และหน้าตั้งค่า Wi-Fi แสดงรหัส 8 หลักเป็น `1234 5678` สถานะแสดงเป็นคำอ่านง่าย เช่น Live, Stale, Off, Sign in หน้าจอวาดใหม่เมื่อข้อมูลหรือนาทีเปลี่ยนเท่านั้น
- `fixtures/*-v1.json`: ข้อมูลสังเคราะห์ `source=mock` ซึ่งหน้าจอระบุ DEMO เสมอ ไม่ใช่ผลจากบัญชีจริง
- `host/`: หน้าต่าง SDL ที่แสดง pixel จาก LVGL flush จริง พร้อมเมาส์แทน touch และการทดสอบโมเดลแบบ deterministic
- `main/`: driver SPI จอ/touch และ runtime Wi-Fi, MQTT, SNTP, HTTP provisioning ของ ESP-IDF

ข้อความบนหน้าจอใช้ฟอนต์ Montserrat 12/14/16/20/28 ที่ LVGL มีให้ ไม่โหลดฟอนต์จากเครือข่าย ภาษา UI เป็นอังกฤษเพราะฟอนต์นี้ไม่มีอักษรไทย host simulator ใช้ LVGL heap 48 KB เท่ากับอุปกรณ์ เพื่อให้หน้าที่ใช้หน่วยความจำเกินพังใน simulator ก่อน

## Simulator บนคอมพิวเตอร์

ต้องมี CMake >=3.20, C/C++ compiler ที่รองรับ C++17, Git และ SDL2 >=2.0.18 ใช้ SDL2 CMake config หรือ `sdl2-config` ได้ ไม่บังคับติดตั้ง pkg-config การ configure ครั้งแรกดึง LVGL tag `v8.3.11` และ cJSON tag `v1.7.19` ผ่าน CMake FetchContent

รันจาก root ของ repository:

```sh
cmake -S firmware/host -B build/firmware-host
cmake --build build/firmware-host
ctest --test-dir build/firmware-host --output-on-failure
./build/firmware-host/quota_simulator --fixtures firmware/fixtures
```

หน้าต่าง native ขนาด 640×480 แสดง framebuffer 320×240 คลิกการ์ด Claude/Codex, ปุ่มกลับ และปุ่มเฟือง และเลื่อนรายละเอียดด้วยเมาส์ กด Escape เพื่อปิด ไม่มีการเรียก provider หรือ broker ใน simulator

```sh
./build/firmware-host/quota_simulator --scenario unknown
./build/firmware-host/quota_simulator --scenario behind --view codex
./build/firmware-host/quota_simulator --scenario expired --view claude
./build/firmware-host/quota_simulator --scenario offline
./build/firmware-host/quota_simulator --scenario reauth
./build/firmware-host/quota_simulator --scenario error
./build/firmware-host/quota_simulator --frames 120 --screenshot /tmp/quota-overview.bmp
```

`--scenario` รองรับ `normal`, `stale`, `ahead`, `behind`, `unknown`, `expired`, `offline`, `reauth`, `error`, `setup` โดยยึดเวลาสังเคราะห์ของ fixture แล้วเดินต่อด้วย monotonic clock ไม่เปลี่ยนนาฬิกาคอมพิวเตอร์ `setup` แสดงหน้าตั้งค่า Wi-Fi ด้วยรหัสสังเคราะห์ `offline` แสดงข้อความ "Cannot join Wi-Fi" ในหน้า Settings `--layout cards|gauges|terminal` เลือก layout ของ Overview `--view` รองรับ `overview|claude|codex|settings|confirm` (`confirm` เปิด Settings แล้วแตะ Reconfigure Wi-Fi เพื่อแสดงหน้ายืนยัน ไม่กดยืนยัน) แตะตำแหน่งเดียวกับนิ้วบนอุปกรณ์ `--frames` ปิดหลังจำนวน frame ที่กำหนด `--screenshot` บันทึก framebuffer ที่ LVGL วาดจริงเป็น BMP; ต้องเปิดไฟล์ตรวจว่าพื้นผิวมีเนื้อหาก่อนอ้างว่าเห็นหน้าจอแล้ว

`quota_model_tests` ใช้ assert ที่ไม่ถูกปิดใน Release build ครอบคลุม duplicate/older payload, topic mismatch, malformed/oversized/deep JSON, UTF-8, missing/null, zero, used เกิน100, duration ที่ไม่ใช่5ชั่วโมง/7วัน, timestamp0, independent provider state, error ที่รักษาข้อมูลเดิม, capped/partial expiry, clock unknown/ahead/behind/backwards และ expired reset ที่ไม่คืน quota เอง

`quota_provision_tests` ตรวจ parser ของ HTTP provisioning โดยใช้ข้อมูลสังเคราะห์เท่านั้น และถูกเพิ่มเข้า CTest เดียวกัน ตรวจ bounds, duplicate fields, URL decoding และรูปแบบ credentials/host/port ก่อนนำไปใช้บน ESP-IDF รวมถึงปฏิเสธฟอร์มที่ยังส่งฟิลด์ `timezone` เดิม `quota_model_tests` ตรวจ `device/config` ด้วย: payload ครบ, บางฟิลด์, layout ไม่รู้จัก, brightness ไม่ใช่จำนวนเต็ม/นอกช่วง, timezone ผิดชุดอักขระหรือยาวเกิน, schema_version ผิดและ duplicate key

## ESP-IDF และ flash 4 MB

ใช้ ESP-IDF **v5.3.3** และ target `esp32` LVGL ถูก pin ใน component manifest เป็น **8.3.11** cJSON, esp-mqtt, Wi-Fi และ SNTP ใช้ component ของ ESP-IDF ไม่ติดตั้ง Arduino/TFT_eSPI

หลังโหลด environment ของ ESP-IDF:

```sh
idf.py -C firmware set-target esp32
idf.py -C firmware menuconfig
idf.py -C firmware build
```

อย่า flash เพื่อทดสอบซอฟต์แวร์โดยอัตโนมัติ ต้องตรวจ controller/pins/calibration และผล build ก่อนให้ผู้ใช้ดำเนินการบนอุปกรณ์จริง

`partitions.csv` รักษา layout 4 MB: NVS `0x9000/0x6000`, PHY `0xf000/0x1000`, factory app `0x10000/0x3F0000` ไม่มี OTA partition และไม่ต้องใช้ PSRAM NVS เก็บ Device setup และ layout/timezone/brightness (namespace `quota`, ไม่เข้ารหัส) ส่วน Wi-Fi library ยังตั้ง `WIFI_STORAGE_RAM` และ `nvs_enable=0` เฟิร์มแวร์เขียน NVS เองเท่านั้น

## Driver ที่ตั้งค่าไว้กับหลักฐานกายภาพ

preset เป็น **ILI9341 SPI landscape 320×240 + XPT2046 แยก SPI bus** ไม่ใช่ driver จำลองและไม่มี no-op ที่อ้างว่าแสดงผลสำเร็จ มีการส่ง initialization command, address window, RGB565 pixel และอ่าน touch controller จริง ค่าใน menuconfig ใช้ปรับ pins, SPI frequency, geometry/orientation และ raw calibration ตาม board จริง

| สัญญาณ | preset GPIO |
|---|---:|
| TFT SCLK / MOSI / MISO | 14 / 13 / 12 |
| TFT CS / DC / reset | 15 / 2 / -1 (ผูกกับ EN) |
| Backlight active-high | 21 |
| Touch SCLK / MOSI / MISO | 25 / 32 / 39 |
| Touch CS / IRQ | 33 / 36 |

calibration เบื้องต้น: raw X 512..3340 ไปแกน Y 0..239 และ raw Y 408..3370 ไปแกน X 0..319 GPIO39/36 เป็น input-only ต้องไม่ย้ายไปใช้งาน output ใช้ SPI bus แยกสำหรับ TFT และ touch ตามหลักฐาน ไม่ใช้ generic CYD preset ที่แชร์ pins

หลักฐานเก่าใน [`../docs/hardware.md`](../docs/hardware.md) ไม่สอดคล้องกันทั้งหมด: บรรทัด16 รายงาน ILI9342-compatible active square240×240 แต่บรรทัด55 รายงาน ILI9341 landscape320×240 ของ Arduino ที่ตรวจภายหลัง ชื่อ controller ยังไม่ยืนยันด้วย ID ดังนั้น preset **ESP-IDF ใหม่นี้ยังไม่ผ่านการพิสูจน์บนบอร์ดจริง** แม้โค้ด driver มีการทำงานจริง ต้องตรวจเต็มพื้นที่จอ สี/byte order/orientation และ touch ทั้ง4มุมด้วยผู้ใช้ หาก controller ไม่รองรับชุดคำสั่ง ILI9341 ต้องเปลี่ยน driver ใน `main/board.cpp`; simulator ไม่พิสูจน์ driver นี้ PSRAM ยังไม่ทราบและไม่ได้เป็น dependency

## Device setup ที่เก็บใน NVS

เมื่อเปิดเครื่องครั้งแรก (หรือหลัง Reconfigure) อุปกรณ์สร้าง AP ชั่วคราวชื่อ `Usage-Setup-<MAC suffix>` แบบ WPA2 พร้อม password สุ่มเลข8หลัก **แสดงบน TFT เท่านั้น** ไม่มีการพิมพ์ password/token ลง serial หรือ MQTT เปิด URL ที่แสดงบนจอจากโทรศัพท์ที่เชื่อม AP

กรอก home Wi-Fi SSID/password, MQTT host/port, username/password สำหรับ subscriber แบบ read-only และ TLS/CA หากใช้ TLS SSID ใช้ชื่อเครือข่ายที่เข้าถึง LAN broker ได้ ห้ามใส่ token ของ provider ลงแบบฟอร์ม TLS ต้องใช้ CA ที่ถูกต้องและตรวจชื่อ host ไม่มีตัวเลือกปิดการตรวจ certificate ฟอร์มมี token ของ setup session เพื่อกันการส่ง form จาก session อื่น token นี้ไม่ใช่ provider token ไม่ถูกเก็บและไม่ส่งลง MQTT ฟอร์มไม่มีช่อง timezone แล้ว: client เก่าที่ส่งฟิลด์ `timezone` มาจะถูกปฏิเสธ

ค่า setup (Wi-Fi SSID/password, MQTT host/port/username/password/TLS/CA) เก็บใน NVS namespace `quota` แบบ **ไม่เข้ารหัส** ตาม [ADR 0002](../docs/adr/0002-device-credentials-in-unencrypted-nvs.md) ผู้ที่เข้าถึงอุปกรณ์ได้ทางกายภาพอ่านได้ด้วย flash dump (NVS/flash encryption ปิดไว้โดยตั้งใจ) หลัง reboot ถ้ามีค่าครบ อุปกรณ์เข้า Connecting ทันทีโดยไม่เปิด AP ถ้าไม่มีหรือไม่ครบ จึงเปิด setup AP เหมือนครั้งแรก นอกจากนี้ NVS เก็บ `timezone`, `layout`, `brightness` ล่าสุดจาก `device/config` (เขียนเฉพาะเมื่อค่าเปลี่ยน)

ค่าที่เพิ่งส่งจากฟอร์ม setup อยู่ใน RAM เท่านั้น และถูกบันทึกลง NVS **หลัง STA ได้ IP สำเร็จเท่านั้น** (`IP_EVENT_STA_GOT_IP`) ถ้าค่าชุดนี้ยังเชื่อม Wi-Fi ไม่ได้ครบ 3 ครั้ง (เช่นพิมพ์ password ผิด) อุปกรณ์ทิ้งค่าชุดนั้น (`clear_sensitive`) กลับสู่ Stage::Setup เปิด setup AP ใหม่พร้อม password 8 หลักชุดใหม่ และแสดง "Could not join Wi-Fi. Check the password and try again." บน TFT ฟอร์มรับค่าใหม่ได้อีกครั้ง (การตัดสินใจนี้คือ `abandon_unsaved_setup` ใน `provision.cpp` ซึ่งมี host test) ค่าที่บันทึกไว้แล้วทำงานต่างออกไป: AP ปิดหลัง STA ได้ IP ถ้าเชื่อม Wi-Fi ไม่ได้ อุปกรณ์ retry แบบ backoff สูงสุด 30 วินาที **ไม่ลบค่าและไม่เปิด AP เอง** หน้า Settings แสดง "Cannot join Wi-Fi. Open Settings > Reconfigure Wi-Fi." และยังแสดง usage ล่าสุดที่มี เมื่อต้องการเริ่ม setup ใหม่ เลื่อนไปท้ายหน้า Settings แตะ **Reconfigure Wi-Fi** แล้วยืนยัน "Erase Wi-Fi and broker settings and start setup?" (ปุ่ม Cancel เด่นเป็น default, ปุ่ม Erase สีแดงอยู่ไกล) การยืนยันลบเฉพาะค่า Wi-Fi/broker ใน NVS (เก็บ timezone/layout/brightness) หยุด STA และ MQTT แล้วเปิด setup AP พร้อม password ใหม่เหมือนบูตครั้งแรก

HTTP provisioning อยู่บน AP ชั่วคราว ไม่ใช่ HTTPS จึงต้องใช้ password AP ที่แสดงเฉพาะหน้าจอและปิด AP เมื่อเสร็จ WPA2 ป้องกันทางวิทยุแต่ไม่แทน TLS ของ MQTT สำหรับ LAN ที่มีผู้ใช้ไม่ไว้วางใจ ใช้ MQTT TLS และ CA จริง `CONFIG_HTTPD_MAX_REQ_HDR_LEN` ยังเป็น 2048 เฟิร์มแวร์ subscribe เฉพาะ:

- `ai-quota/v1/providers/claude`
- `ai-quota/v1/providers/codex`
- `ai-quota/v1/server/status`
- `ai-quota/v1/device/config` (retained, schema_version 1)

ใช้ QoS1 และรับ retained ข้อมูลหลัง reconnect `device/config` ถอดรหัสทีละฟิลด์: `layout` (`cards|gauges|terminal`), `timezone` (POSIX TZ 1..64 ตัวอักษรจาก `A-Za-z0-9+-:,./<>`) และ `brightness` (จำนวนเต็ม ถูก clamp 10..100) ฟิลด์ที่ไม่รู้จัก/ผิดชนิด/ไม่ผ่านการตรวจจะคงค่าเดิม ไม่ปฏิเสธทั้ง payload เว้นแต่ `schema_version` ไม่ใช่ 1 หรือ JSON เสีย อุปกรณ์ publish ได้เพียง `ai-quota/v1/device/status` (retained QoS1): `{"state":"online"}` หลังเชื่อม broker และ Last Will `{"state":"offline"}` เมื่อหลุดโดยไม่ disconnect ปกติ (ตอน Reconfigure จะ publish `offline` เองก่อนปิด) ไม่ publish คำสั่ง reset/purchase/inference หรือ credentials

ความสว่างจอควบคุมด้วย LEDC PWM บน GPIO ของ backlight (`CONFIG_QUOTA_LCD_BL`, 5 kHz 10 บิต) layout `cards` (การ์ดคู่), `gauges` (วงแหวน `lv_arc` สองวง: % ที่เหลือของ window สั้นอยู่กลางวง, window รายสัปดาห์ด้านล่าง) และ `terminal` (ฟอนต์ `lv_font_unscii_8` เปิดด้วย `CONFIG_LV_FONT_UNSCII_8`; หนึ่งบรรทัดต่อ window) ใช้ top bar, การแตะเข้าหน้ารายละเอียด และหน้า Settings ร่วมกัน

## เวลาและความสดของข้อมูล

UTC เป็น Unix seconds แบบ absolute; timezone ใช้เพื่อแสดงผลเท่านั้น และมาจาก `device/config` ของ server (ฟิลด์ `timezone`) ไม่มีช่องกรอกในฟอร์ม setup แล้ว ค่าล่าสุดที่รับถูกเก็บใน NVS ถ้ายังไม่เคยได้รับใช้ `CONFIG_QUOTA_TIMEZONE` (default `ICT-7` คือ POSIX TZ สำหรับ Bangkok UTC+7 ไม่ใช่ IANA string `Asia/Bangkok` และเครื่องหมายของ POSIX กลับจาก offset ที่คนใช้ทั่วไป)

ก่อน SNTP sync ที่เชื่อถือได้ หน้าจอแสดง `Syncing clock` และไม่เดา countdown จากเวลาที่รับ MQTT หลัง sync เวลา UTC เดินด้วย local monotonic clock ระหว่างการ sync ครั้งถัดไป การแก้นาฬิกาถอยหลังไม่ใช้ elapsed เวลาติดลบ

SNTP ไม่ตรวจลายเซ็นหรือรับรอง server ทาง cryptography คำว่าเวลาเชื่อถือได้ใน UI หมายถึงได้รับ callback การ sync ไม่ใช่ proof ของแหล่งเวลา ควรตั้ง NTP server ใน LAN ที่ไว้ใจได้ MQTT TLS รอการ sync ก่อนเชื่อมต่อและตรวจ CA/hostname โดยไม่ bypass verification

- Freshness คำนวณจาก **fetched_at** เท่านั้น ไม่ใช้ published_at ใหม่จาก error เพื่อทำให้ข้อมูลเก่าดูสด
- แต่ละ provider มี snapshot และ state แยกกัน Failure ของ Claude ไม่แก้ Codex
- payload จาก source เดียวกันที่ซ้ำหรือ published_at เก่าจะถูกละเลย และ fetched_at เก่ากว่าจะไม่ทับข้อมูลใหม่ Empty error update รักษา windows/count/fetched_at เดิม ข้อมูล live แทน DEMO ทั้งชุดโดยไม่ยก mock data มาแสดงเป็น live และ mock ที่มาภายหลังไม่ทับ live
- ถ้า fetched_at นำเวลาท้องถิ่นเกิน60วินาที แสดง `CLOCK SKEW / STALE` ไม่อ้างข้อมูลสด ถ้าไม่มีเวลา/fetched_at จะแสดงความไม่ทราบอย่างชัดเจน
- remaining มาจาก `clamp(100-used,0,100)` ที่ตรวจตรงกับ payload ใช้0ได้; null/missing คือ unknown ไม่ใช่0
- reset/expiry countdown คือ `max(0,target-now)` เมื่อ quota reset ผ่านไป แสดง `Awaiting provider refresh` ไม่เพิ่ม remaining เป็น100% เอง
- banked count เป็นค่าทางการจาก server ไม่คำนวณจากจำนวนรายละเอียดและไม่หัก credit เองเมื่อเวลาผ่าน รายละเอียด complete/partial/unavailable คงไว้ แสดงเฉพาะ expiry ที่รู้จริง null expiry ไม่แปลว่า nonexpiring; nonexpiring แสดงได้เฉพาะหลักฐาน explicit
- `OFFLINE`, `STALE`, `REAUTH REQUIRED`, `RATE LIMITED`, `UNAVAILABLE`, `ERROR` และ `DEMO` แสดงแยกจากตัวเลขเดิม เมื่อ reauth ให้ทำขั้นตอนฝั่ง server ตามคู่มือ auth ไม่ป้อน provider token ลงอุปกรณ์

## ขอบเขตการพิสูจน์

คำสั่ง build/test/simulator ด้านบนเป็นขั้นตอนตรวจที่รันได้ ไม่ใช่คำกล่าวว่ารันผ่านแล้ว งาน implementation นี้ไม่ build/test/install/flash และไม่เชื่อมบัญชีจริง ผู้รวมงานต้องบันทึกผล host test และเปิด simulator ตรวจ surface จริงแยกจากผล ESP-IDF compile ผู้ใช้ต้องยืนยัน driver/touch/Wi-Fi/TLS/reconnect/SNTP บน hardware ก่อนเรียกว่าพร้อมใช้กับบอร์ดจริง
