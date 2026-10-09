# MQTT protocol v1

โปรโตคอลนี้ส่งโควตาสมาชิก ไม่ใช่ค่าใช้จ่าย API และไม่อนุญาตให้สั่งซื้อหรือใช้ banked reset อุปกรณ์ไม่มี OAuth token และไม่เรียก API ของผู้ให้บริการ

## Topics และสิทธิ์

| Topic | ผู้เขียน | ผู้อ่าน |
|---|---|---|
| `ai-quota/v1/providers/codex` | `quota-server` | `esp32` |
| `ai-quota/v1/providers/claude` | `quota-server` | `esp32` |
| `ai-quota/v1/server/status` | `quota-server` | `esp32` |
| `ai-quota/v1/device/config` | `quota-server` | `esp32` |
| `ai-quota/v1/device/status` | `esp32` | `quota-server` |

ACL เป็นแบบ allow-list: `quota-server` เขียนได้เฉพาะสี่ topic แรก (providers, server/status, device/config) และอ่านได้เฉพาะ `device/status`; `esp32` อ่านสี่ topic แรกและ **เขียนได้เฉพาะ `device/status`** ไม่มีสิทธิ์เขียน topic อื่นเลย ไฟล์ที่ใช้จริงคือ `deploy/mosquitto/acl`

ทุกข้อความใช้ QoS 1 และ retain; Mosquitto เปิด persistence อุปกรณ์ที่ subscribe ใหม่ได้รับสถานะล่าสุดโดยไม่รอรอบ polling QoS 1 ส่งข้อความซ้ำได้ จึงใช้ snapshot แทน event สำหรับเปลี่ยนยอดโควตา

Server status เป็น `{"state":"online"}` หรือ `{"state":"offline"}` ข้อความ offline เป็น Last Will และส่งเมื่อปิดอย่างปกติด้วย Broker เก็บ snapshot ของผู้ให้บริการไว้แม้ server offline อุปกรณ์ต้องพิจารณาเวลาและสถานะ server แยกกัน

## Device config และ Device status

`ai-quota/v1/device/config` เป็นค่าที่ผู้ใช้ตั้งผ่าน Admin app (หน้า Device) `quota-server` เขียนด้วย QoS 1 + retain ทุกครั้งที่ `PUT /api/device` สำเร็จ และเขียนซ้ำจาก `device.json` ทุกครั้งที่ server เริ่มทำงาน (หรือค่าเริ่มต้นถ้ายังไม่มีไฟล์) อุปกรณ์จึงได้ค่าล่าสุดทันทีที่ subscribe

```json
{"schema_version":1,"layout":"cards","timezone":"ICT-7","brightness":80}
```

| Field | กติกา |
|---|---|
| `schema_version` | ต้องเป็น `1` |
| `layout` | `cards`, `gauges` หรือ `terminal`; ค่าอื่นให้คงค่าเดิมบนอุปกรณ์ |
| `timezone` | POSIX TZ ยาว 1–64 ตัวอักษรจาก `A-Z a-z 0-9 + - : , . / < >` เช่น `ICT-7`, `CET-1CEST,M3.5.0,M10.5.0/3`; ผิดกติกาให้คงค่าเดิม |
| `brightness` | จำนวนเต็ม 10–100; อุปกรณ์ clamp ค่านอกช่วงที่เป็นจำนวนเต็ม ส่วนค่าที่ไม่ใช่จำนวนเต็มให้ละทิ้ง |

อุปกรณ์ decode ทีละ field: field ที่ไม่รู้จักหรือไม่ถูกต้องคงค่าเดิม ไม่ปฏิเสธทั้ง payload และเก็บเฉพาะค่าที่ยอมรับลง NVS เมื่อมีการเปลี่ยนแปลง `schema_version` อื่นไม่ถูกยอมรับ ค่าเริ่มต้นก่อนได้รับ config คือ `cards` / `ICT-7` / `80` Server ตรวจค่าเดียวกันนี้ก่อนบันทึก (400 ถ้าผิด) จึงไม่ส่งค่าผิดกติกาออกไป

`ai-quota/v1/device/status` เขียนโดย `esp32` เท่านั้น ส่ง `{"state":"online"}` (QoS 1, retain) หลัง MQTT เชื่อมต่อสำเร็จ และตั้ง Last Will เป็น `{"state":"offline"}` (QoS 1, retain) เพื่อให้ broker ประกาศเมื่ออุปกรณ์หลุดโดยไม่บอกลา `quota-server` subscribe topic นี้และรายงานผ่าน `GET /api/status` เป็น `device.state`: `unknown` จนกว่าจะได้ข้อความแรก จากนั้น `online`/`offline` ตามที่ได้รับ payload อื่นที่ไม่ใช่สองแบบนี้ถูกละเว้น ไม่ถูกตีความเป็น offline

## Snapshot

ชนิดข้อมูลอ้างอิงอยู่ใน `crates/quota-core/src/lib.rs` และ decoder อุปกรณ์อยู่ใน `firmware/` Payload สูงสุด **16,384 bytes**, usage windows สูงสุด **32**, วันหมดอายุที่ทราบสูงสุด **64**; `limit_id`, `key`, `label` สูงสุด 64 UTF-8 bytes ไม่มี control characters ห้ามส่งข้อมูลดิบที่มี token, account ID, email หรือ opaque credit ID

| Field | ความหมาย |
|---|---|
| `schema_version` | ต้องเป็น `1`; รุ่นอื่นต้องถูกปฏิเสธ |
| `provider` | `codex` หรือ `claude`; ต้องตรงกับ topic |
| `state` | สถานะการดึงข้อมูลล่าสุด |
| `source` | `provider` หรือ `mock`; ถ้าขาด field ให้ถือเป็น `provider`; UI ต้องติดป้าย DEMO เมื่อเป็น mock |
| `fetched_at` | UTC Unix seconds ของข้อมูลจากการดึงสำเร็จครั้งล่าสุด; `null` เมื่อยังไม่มี |
| `published_at` | UTC Unix seconds ที่เผยแพร่ snapshot นี้ ไม่ใช่เวลาของข้อมูลโควตา |
| `stale_after_seconds` | อายุสูงสุดก่อนติดป้าย stale; ต้องมากกว่า 0 |
| `windows` | รายการ windows ของผู้ให้บริการ ไม่รวมโควตาสองเจ้าเป็นเปอร์เซ็นต์เดียว |
| `banked_resets` | Codex reset-credit summary หรือ `null`; Claude ใช้ `null` |
| `error_code` | safe enum หรือ `null`; ไม่ส่งข้อความ error จาก upstream |

ค่าที่เป็น nullable รับ `null` หรือไม่ส่ง field ก็ได้ ทั้งสองกรณีหมายถึงไม่ทราบ ไม่ใช่เลขศูนย์ ฟิลด์อื่นต้องมีตาม schema ส่วน field ใหม่ที่ไม่รู้จักให้ข้ามได้โดยไม่เปลี่ยนความหมายของ field เดิม

### Usage window

`limit_id` ระบุ bucket, `key` ระบุ window ภายใน bucket และ `label` ใช้แสดงผล คู่ `limit_id` + `key` ต้องไม่ซ้ำ `duration_minutes` เก็บระยะจริงหรือ `null`; ไม่บังคับว่าทุกบัญชีต้องมี 300 หรือ 10,080 นาที

`used_percent` คือใช้ไปแล้ว และ `remaining_percent = clamp(100 - used_percent, 0, 100)` เฉพาะเมื่อทราบ usage ทั้งคู่เป็น `null` ถ้าไม่ทราบ ค่า used ต้องเป็นตัวเลข finite ไม่ติดลบ; remaining ไม่ใช่การประมาณจาก tokens

`resets_at` คือ **UTC Unix seconds** ของ window renewal หรือ `null` ไม่ใช่ `expiresAt` ในไฟล์ Claude ซึ่งเป็น milliseconds และไม่ใช่วันหมดอายุของ banked reset

### Banked resets

- `available_count`: จำนวนที่ upstream ยืนยัน หรือ `null` จำนวนนี้เป็น authoritative ไม่คำนวณจากจำนวนแถวรายละเอียด
- `details_status`: `complete`, `partial` หรือ `unavailable`
- `known_expirations_at`: วันหมดอายุที่ส่งกลับมาจริงเท่านั้น รายการว่างไม่ได้แปลว่าทุกเครดิตไม่มีวันหมดอายุ
- `known_nonexpiring_count`: เพิ่มเฉพาะกรณี upstream ยืนยันว่าไม่หมดอายุ ไม่อนุมานจากวันหมดอายุที่หายไป
- วันที่ใกล้ที่สุดในรายการ partial ต้องเขียนว่า **Known expiry** ไม่ใช่รับประกันว่าเป็นเครดิตที่หมดอายุใกล้ที่สุดทั้งหมด
- การไม่มี expiry ไม่เท่ากับ epoch 0 และไม่แสดงเป็นหมดอายุทันที

## States และ errors

| State | ความหมาย |
|---|---|
| `ok` | ดึงข้อมูลสำเร็จ มี `fetched_at` และไม่มี error |
| `stale` | ข้อมูลจาก cache ที่ยังไม่ยืนยันใหม่ หรือข้อมูลเก่าเกินเกณฑ์ |
| `reauth_required` | ต้อง login ใหม่จาก hard authentication failure |
| `rate_limited` | upstream ตอบ 429; รอตาม Retry-After/backoff |
| `unavailable` | ยังดึงไม่ได้, disabled, network, timeout, process หรือ policy restriction |
| `error` | schema/parse หรือ cache error |

Safe error codes: `disabled`, `unauthenticated`, `auth_rejected`, `access_denied`, `rate_limited`, `network`, `timeout`, `invalid_response`, `process_unavailable`, `unsupported`, `cache`, `upstream` รหัส `upstream` แยก server error ของผู้ให้บริการออกจาก network failure

เมื่อ error ต้องเก็บ windows และ `fetched_at` จากการดึงสำเร็จเดิม แต่เปลี่ยน `state`, `error_code` และ `published_at` การเผยแพร่ error ซ้ำไม่ทำให้ข้อมูลเดิมดูใหม่ขึ้น บันทึก cache เฉพาะข้อมูลสำเร็จ และแสดงข้อมูลที่ restore เป็น stale จนดึงสำเร็จใหม่ Cache mock ใช้แทนข้อมูลจริงไม่ได้

## เวลาและการ reconnect

อุปกรณ์แสดง `Syncing clock` จน NTP ยืนยันเวลา ใช้เวลาฐาน UTC กับ monotonic ticks ระหว่าง sync และแปลง timezone เฉพาะตอนแสดงวันเวลา timezone มาจาก `device/config` (ตั้งในหน้า Device ของ Admin app) ค่าเริ่มต้นก่อนได้รับค่า คือ POSIX `ICT-7` และอุปกรณ์เก็บค่าล่าสุดใน NVS ไว้ใช้หลัง reboot ฟอร์มตั้งค่า Wi-Fi ไม่มีช่อง timezone แล้ว

Countdown คือ `max(0, target - now)` แสดงวัน/ชั่วโมง/นาที เมื่อ reset countdown เป็นศูนย์ให้แสดง `Awaiting provider refresh` จนได้รับข้อมูล authoritative ใหม่ **ห้ามตั้ง remaining เป็น 100% เอง**

เมื่อ `now - fetched_at > stale_after_seconds` อุปกรณ์ติดป้าย stale แม้ข้อความ retained เคยมี state ok ถ้า fetched_at อยู่ในอนาคตเมื่อเทียบกับ clock ให้แสดง clock inconsistency ไม่แกล้งถือว่าข้อมูล fresh ข้อความซ้ำไม่เพิ่ม quota และข้อความเก่าต้องไม่ทำให้ state ใหม่ถอยหลัง

## Security

MQTT แบบ plaintext แม้มี password ก็ไม่เข้ารหัส ใช้เฉพาะ trusted LAN หรือเปิด TLS พร้อมตรวจ CA/hostname เมื่อต้องข้าม segment ที่ไม่เชื่อถือ ไม่เปิด broker, Codex App Server หรือ credential services ผ่าน Cloudflare Tunnel การตั้งค่า auth/ACL อยู่ใน `deploy/` และขั้นตอนสร้างรหัสอยู่ใน [OPERATIONS.md](OPERATIONS.md)
