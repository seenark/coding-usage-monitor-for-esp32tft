# การทดสอบและขอบเขตหลักฐาน

การทดสอบอัตโนมัติใช้ข้อมูลจำลองและ HTTP server/MQTT broker ภายในเครื่อง ไม่เข้าสู่บัญชีจริง ไม่ทำ inference ไม่ใช้หรือซื้อ banked reset และไม่แฟลช ESP32 Fixtures ที่มี credential fields เป็นค่าทดสอบสังเคราะห์ ไม่ใช่ credentials จริง ห้ามนำข้อมูลบัญชีจริงมาแทน fixture หรือแนบ logs ของ provider response

## Rust

จากราก repository:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

`codex_protocol` ใช้ child process จำลองเพื่อพิสูจน์ handshake/initialized/IDs/notifications/read-only methods และ restart โดยไม่ต้องติดตั้งหรือ login Codex สำหรับการทดสอบนี้:

```sh
cargo test -p quota-server --test codex_protocol
```

Claude tests เรียก adapter จริงกับ HTTP server loopback และ credentials สังเคราะห์ใน temporary directory ตรวจ ISO reset timestamps, expiry milliseconds, scope/policy/errors, refresh ที่จำกัดจำนวนครั้ง, token rotation/unknown fields/permissions และการยกเลิก request โดยไม่ให้เกิด writer ซ้อน

`admin_api` (ใน `cargo test --workspace`, ไม่ต้องใช้ broker หรือ Docker) รัน HTTP server จริงบน loopback พร้อม scheduler จริงกับ collector สคริปต์ ตรวจทุก route ของ Admin app: รูป `400 {"error":"invalid","fields":...}`, 404/405/413/415, ค่าขอบเขตของ settings (59/60/3600/3601, backoff น้อยกว่า interval) และของ Device (layout, timezone, brightness 9/10/100/101), การเขียนไฟล์ `0600` แบบ atomic, ค่าในไฟล์ชนะ env, การเปิด/ปิด provider ขณะรัน, `409 disabled`, `429` ขณะ backoff ทั้งจากการกดซ้ำและหลังลด interval, SPA fallback กับ cache header และข้อห้ามว่าไม่มี route ใดคืนรหัสผ่าน MQTT ของ server (รหัสผ่านของอุปกรณ์อยู่ที่ `/api/device-setup` เท่านั้น) หน่วย: ขอบเขต settings, ลำดับ env/ไฟล์, backoff/Retry-After และ parser ของ `device/status` อยู่ใน `crates/quota-server/src` ชุด Python `python3 -m unittest discover -s deploy -p 'test_setup.py'` ตรวจ setup, ACL (esp32 เขียนได้เฉพาะ `device/status`) และ `.env.example`

Core/cache tests ตรวจ remaining จาก known usage, unknown usage, stale/clock mismatch, countdown ที่หมดเวลา, provider/topic mismatch, schema/size bounds, restore cache และห้าม restore mock เป็นข้อมูลจริง

## MQTT แบบ end-to-end

ทดสอบบน Docker daemon ที่ทำงานอยู่:

```sh
./scripts/smoke.sh
```

สคริปต์ใช้ broker และ state สำหรับทดสอบแยกจาก installation จริง พร้อม generated passwords ที่ไม่แสดงค่า ไม่เปลี่ยน provider auth ของคุณ และไม่ลบ Docker volumes หรือ state ของ installation อื่น

สิ่งที่ต้องพิสูจน์: mock server ส่ง provider snapshots QoS 1/retain, subscriber ใหม่ได้รับข้อมูล retained, ACL (อุปกรณ์อ่านอย่างเดียวบน providers/server status/device config และเขียนได้เฉพาะ `device/status`; server เขียนได้เฉพาะ topic ของตน), `device/config` retained และ server ติดตามสถานะอุปกรณ์จาก `device/status`, server status Last Will, การเชื่อมใหม่ของ publisher, provider failure isolation, cache ของ last-success ไม่เปลี่ยน fetched_at หลัง failure และ broker persistence หลัง restart Integration tests ที่ต้องใช้ broker ถูก ignore ใน `cargo test --workspace`; smoke เรียกชุดนี้อย่างชัดเจน

ข้อมูล mock ต้องมี `source=mock` และ UI แสดง DEMO การเห็นตัวเลขจาก mock ไม่ใช่หลักฐานว่าสามารถดึงโควตาสมาชิกจริงได้

## Firmware host tests และ LVGL simulator

ดูคำสั่ง build และ scenarios ที่รองรับจริงใน [firmware/README.md](../firmware/README.md) Host tests ต้องตรวจ malformed/oversized payload, nullable windows/expiry, duplicates/old snapshots, trusted/unknown clock, stale thresholds, expired reset และ known expiry แบบ partial

Simulator แสดง LVGL screens จริงด้วยข้อมูลสังเคราะห์: Overview ของสองผู้ให้บริการ, หน้า Claude, หน้า Codex, model details และสถานะผิดพลาด/เก่า ไม่ใช่ driver test ของบอร์ด ต้องเปิด simulator และดู surface จริงเพิ่มเติมจาก unit tests

## Read-only acceptance ด้วยบัญชีของเจ้าของ

ขั้นตอนนี้ต้องทำด้วยตนเองหลัง [AUTH.md](AUTH.md) ไม่รันใน CI หรือโดย coding agent และบันทึกเฉพาะสถานะ ไม่บันทึก token/email/บัญชี:

1. Login CLI ใน isolated persistent mounts และตรวจ status โดยไม่เปิด credential file
2. Recreate auth helper แล้วตรวจว่ายัง login อยู่ Restart server โดยไม่เปลี่ยน mount
3. Codex: เปรียบเทียบ remaining และวัน reset กับ provider Usage UI โดยเผื่อ poll interval ตรวจ banked count และทำให้ unknown/partial expiry เห็นชัด
4. Claude: เปิด opt-in หลังยอมรับความเสี่ยงและ login เท่านั้น ถ้า endpoint/account ปฏิเสธต้องแสดง error โดย Codex ยังทำงาน ห้าม workaround หรือเปลี่ยน API billing
5. ปิด MacBook แล้วตรวจว่า homelab ยัง polling และ publish ได้
6. ปิด server แล้วตรวจ retained offline status; retained quotas เดิมต้องแสดง stale ตามอายุ ไม่ใช่ศูนย์

## Acceptance บน ESP32 จริง

ต้องยืนยันรุ่นบอร์ด/controller/pins/active geometry และ touch calibration ก่อน flash Experimental preset ใน firmware ไม่ใช่หลักฐาน driver ที่ผ่านบอร์ดของคุณ

หลังเลือก hardware profile ที่ตรงกันแล้วจึงตรวจ: Overview แสดงสองเจ้า, touch navigation, Wi-Fi setup password เลข 8 หลักที่แสดงเฉพาะ TFT, provisioning ผ่านมือถือ, MQTT retained reconnect, clock sync, countdown/stale และ reboot behavior ของ RAM-only credentials ไม่มีการเปิด Secure Boot/flash encryption หรือ erase keys ในขั้นตอนนี้

## สิ่งที่ผลอัตโนมัติไม่ยืนยัน

- สิทธิ์และผล quota API ของบัญชีจริง รวมถึงความเข้ากันได้ของ private Claude endpoint
- CLI browser/device login และ persistence ของ token หลัง refresh บนบัญชีจริง
- ความตรงของข้อมูลกับ Usage UI และการ polling เมื่อ MacBook ปิดจริง
- controller, touch, calibration และ driver บนบอร์ดจริง
- ความปลอดภัยของ LAN และ firewall ของ installation ปลายทาง

ผลรันที่ตรวจจริงจะรายงานแยกจากรายการเหล่านี้ ห้ามตีความ compile, fixture หรือ simulator ว่าเป็น live-provider/hardware acceptance
