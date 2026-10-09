# การติดตั้งและดูแล AI Quota Monitor

## ขอบเขตและสิ่งที่ต้องมี

ใช้โฮสต์ Linux `linux/amd64` หรือ `linux/arm64` พร้อม Docker Engine, Docker Compose v2 และ Python 3 บน macOS ใช้ Docker Desktop ได้สำหรับ mock และการพัฒนา ส่วนการยืนยันตัวตนอยู่ใน Linux helper ไม่ใช้ Keychain ของ macOS เซิร์ฟเวอร์รัน Rust และ Mosquitto บน LAN ไม่มี database หรือ public tunnel

ยังไม่ได้ตรวจบัญชีจริงหรือฮาร์ดแวร์จริงระหว่างพัฒนา การเข้าสู่ระบบ ทดสอบอุปกรณ์ และเปิด Claude private usage เป็นขั้นตอนของผู้ใช้ รายละเอียดอยู่ใน [AUTH.md](AUTH.md)

## สร้างค่าตั้งและพื้นที่ส่วนตัว

จากราก repository รัน:

```sh
./scripts/setup.sh
```

คำสั่งสร้าง `deploy/.env` จาก `.env.example` ตั้ง UID/GID เป็นบัญชีโฮสต์ สร้าง auth/cache/broker/secrets เป็น `0700` และสุ่ม MQTT password แยกต่อผู้ใช้: `quota-server` ใช้ hex 256 บิต ส่วน `esp32` ใช้ตัวเลขสุ่ม 12 หลัก (ประมาณ 40 บิต) เพื่อให้พิมพ์บนแป้นตัวเลขมือถือได้ บัญชีนี้อ่านได้เฉพาะ topic โควตากับ `device/config` และเขียนได้เฉพาะ `device/status` ของตัวเองบน LAN ไฟล์เป็น `0600` ไม่แสดง password และไม่ส่ง password ใน argv หรือ env ใช้ `mosquitto_passwd -U` แปลงไฟล์ชั่วคราวที่ถูกป้องกันและไม่มีเครือข่าย แล้วติดตั้งเฉพาะ hash ให้ broker เมื่อล้มเหลวไฟล์ plaintext สำหรับแปลงจะถูกลบ รหัสผ่านเดิมไม่เปลี่ยนเมื่อรัน setup ซ้ำ หากต้องการเปลี่ยนรหัส `esp32` ให้ลบ `deploy/state/secrets/mqtt-device-password` รัน setup แล้ว restart `mosquitto` จากนั้นตั้งค่าอุปกรณ์ใหม่

หากต้องการ UID/GID ที่กำหนดเอง ให้ตั้งก่อนการสร้าง `.env` เช่น `HOST_UID=1000 HOST_GID=1000 ./scripts/setup.sh` ผู้รันต้องมีสิทธิ์สร้างไฟล์และเปลี่ยนเจ้าของเป็น ID นั้น ID ทั้งสองต้องไม่เป็น `0` ถ้า `.env` มีอยู่แล้ว ให้แก้ ID ในไฟล์ก่อนและจัดเจ้าของพื้นที่ auth/cache เดิมให้ตรงกัน ห้ามใช้ `chmod 777` แก้ปัญหา

`STATE_DIR=./state` แปลว่า `deploy/state` เพราะ Compose resolve bind mount จากไดเรกทอรีของ `deploy/compose.yaml` ไม่ใช่ working directory สคริปต์ resolve จากตำแหน่งของตนเองและเรียก Compose รูปแบบเดียวกันเสมอ รองรับ absolute state path ต้องเป็นไดเรกทอรีเฉพาะ ไม่ใช่ root หรือราก repository และไม่ผ่าน symlink หากเปลี่ยนพาธให้อยู่ภายใน repository นอก `deploy/state` ต้องเพิ่ม ignore ที่ตรงกันก่อนใช้งาน ค่า `.env` ต้องเป็น `KEY=value` แบบ literal ไม่มี quote, variable expansion หรือคำสั่ง shell

สำหรับ smoke test แยกพื้นที่ ใช้ `QUOTA_ENV_FILE=/absolute/path/test.env` และ `COMPOSE_PROJECT_NAME=ชื่อไม่ซ้ำ` กับสคริปต์ setup/subscriber และทุกคำสั่ง Compose ตั้ง `STATE_DIR` ใน test env เป็น absolute temp path วิธีนี้ไม่อ่านหรือแก้ `deploy/.env` และ credentials ของชุดจริง ห้ามให้ค่าพาธ/UID/GID ใน shell environment ขัดกับ test env

Compose ส่งเฉพาะชื่อไฟล์ secret ให้ Rust server ไม่ส่งค่า secret เป็น environment broker อ่าน hash file เท่านั้น subscriber อ่าน password ผ่านไฟล์ config ของ Mosquitto ที่ mount แบบอ่านอย่างเดียว อุปกรณ์ใช้ผู้ใช้ `esp32` ที่อ่านได้สี่ topic (providers สอง, server/status, device/config) และเขียนได้เฉพาะ `device/status` ไม่ใช้บัญชี `quota-server` ซึ่งเขียน providers/server status/device config และอ่านได้เฉพาะ `device/status` ตาราง ACL เต็มอยู่ใน [MQTT_PROTOCOL.md](MQTT_PROTOCOL.md)

ค่าตั้งเริ่มต้น:

- `MQTT_BIND_ADDRESS=127.0.0.1` และ port `1883` ปลอดภัยสำหรับทดสอบภายในโฮสต์
- `CODEX_ENABLED=true`, `CLAUDE_PRIVATE_USAGE_ENABLED=false`
- Refresh interval ต่อ provider: Claude `600` วินาที (`CLAUDE_POLL_INTERVAL_SECONDS`), Codex `300` วินาที (`CODEX_POLL_INTERVAL_SECONDS`) ช่วงที่ยอมรับ 60–3600; timeout `12` วินาที, stale `900` วินาที และ backoff สูงสุด `1800` วินาที (ต้องไม่น้อยกว่า interval ใดเลย)
- Admin app ฟังที่ `ADMIN_BIND_ADDRESS=127.0.0.1` พอร์ต `ADMIN_PORT=8080` (ดูหัวข้อ Admin app)
- `CODEX_HOME=/auth/codex`, `CLAUDE_CONFIG_DIR=/auth/claude`, cache `/var/lib/quota-server` อ่านเขียนได้
- เซิร์ฟเวอร์และ helper ใช้ non-root UID/GID เดียวกัน root filesystem อ่านอย่างเดียว

## เริ่ม mock โดยไม่ใช้ credentials

```sh
./scripts/setup.sh
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live stop quota-server
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock up -d --build mosquitto quota-server-mock
./scripts/subscriber.sh 3 30
```

subscriber ต้องได้รับ JSON สำหรับ Codex, Claude และ server status ข้อความ provider มี `source="mock"` และ `schema_version=1` ข้อมูลนี้เป็น demo ไม่ใช่โควตาจริง mock ไม่มี auth mount และใช้ `mock-cache` แยกจาก live cache ไม่จำเป็นต้อง login และไม่เรียกบริการผู้ให้บริการ ห้ามเริ่ม live กับ mock พร้อมกัน เพราะทั้งคู่ publish ไป topic เดียวกัน

## เริ่มข้อมูลจริง

ทำ auth ตาม [AUTH.md](AUTH.md) ก่อน:

```sh
./scripts/auth.sh codex login
./scripts/auth.sh codex status
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock stop quota-server-mock
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live up -d --build mosquitto quota-server
./scripts/subscriber.sh 3 30
```

ค่าเริ่มต้น Claude ยังปิดอยู่ ข้อความ Claude แสดง `unavailable` และ `error_code="disabled"` ไม่ได้หมายความว่า Codex ล้มเหลว หากเลือกเปิด Claude ต้องเข้าสู่ระบบแบบ subscription และยอมรับความเสี่ยง private endpoint ก่อนตั้ง `CLAUDE_PRIVATE_USAGE_ENABLED=true` `CLAUDE_PRIVATE_USAGE_ENABLED` และตัวเลขใน env เป็นเพียงค่าเริ่มต้น: เมื่อมี `settings.json` แล้วค่าในไฟล์ชนะ (เปิด/ปิด Claude ได้ในหน้า Server settings โดยไม่ restart) ถ้าแก้ env ให้ใช้ `up -d --force-recreate quota-server` ไม่ใช้เพียง `restart` เพราะ restart ไม่โหลด env ใหม่

ข้อความ provider publish QoS 1 แบบ retained เวลา reset และวันหมดอายุเป็น UTC Unix seconds ไม่ใช้เวลาเครื่องเซิร์ฟเวอร์สร้างโควตาใหม่ เซิร์ฟเวอร์รักษาข้อมูลล่าสุดที่สำเร็จเมื่อ provider ล้มเหลว โดยไม่เปลี่ยน `fetched_at` ให้ดู `state`, `error_code`, `published_at` และ `fetched_at` ร่วมกัน

## เชื่อม ESP32 จาก LAN

เปลี่ยน `MQTT_BIND_ADDRESS` เป็น IPv4 ส่วนตัวที่เป็นของโฮสต์จริง เช่น `192.168.1.10` ไม่ใช้ `0.0.0.0` และไม่ตั้งเป็น public address จากนั้นรัน setup เพื่อตรวจค่าตั้ง และ recreate broker:

```sh
./scripts/setup.sh
docker compose --env-file deploy/.env -f deploy/compose.yaml up -d --force-recreate mosquitto
```

Firewall อนุญาต port เฉพาะ subnet อุปกรณ์ ห้ามทำ router port forwarding, public tunnel หรือให้ broker เข้าถึงจากอินเทอร์เน็ต อุปกรณ์ใช้ MQTT host เป็น LAN IP, port `1883`, username `esp32` และ password ตัวเลข 12 หลักจากไฟล์ส่วนตัว `deploy/state/secrets/mqtt-device-password` ดูรหัสใน terminal ของคุณเองด้วย `cat` แล้วพิมพ์ลงฟอร์มบนมือถือ อย่าวางรหัสลงแชท ticket หรือ log ที่บันทึกไว้ และไม่ให้ credentials ของ Codex/Claude เข้า firmware

### ความเสี่ยง plaintext และ TLS

Compose ที่ให้มาฟัง MQTT plaintext บน LAN แม้ broker ใช้ authentication และ ACL แต่ผู้ที่ดักฟัง LAN ได้อาจอ่าน password และข้อความโควตา การ bind loopback ไม่เข้ารหัสเมื่อคุณเปลี่ยนเป็น LAN ใช้เครือข่ายส่วนตัวที่เชื่อถือได้หรือเพิ่ม TLS ก่อนใช้เครือข่ายร่วมกัน

Rust server รองรับ `MQTT_TLS=true`, `MQTT_CA_FILE` และ `MQTT_PORT` พร้อมตรวจ CA แต่ template ไม่ provision certificate อัตโนมัติ หากเปิด TLS ต้องปรับ Mosquitto listener, `cafile`, `certfile`, `keyfile`, port mapping และ readonly certificate mounts รวมทั้งตั้ง CA และ hostname ที่ตรงกับ certificate ให้ subscriber และอุปกรณ์ด้วย ห้ามปิดการตรวจ certificate หรือใช้ TLS เฉพาะ server แล้วปล่อยอุปกรณ์ plaintext คู่มือทางการ: [Mosquitto TLS configuration](https://mosquitto.org/man/mosquitto-conf-5.html)

## Admin app และ Server settings

Admin app คือหน้าเว็บ + JSON API พอร์ตเดียวจาก `quota-server` (หน้า Overview, Claude, Codex, Server settings, Device, Device setup) container ฟัง `0.0.0.0:8080` ภายใน network ของ Compose และ Compose publish เป็น `${ADMIN_BIND_ADDRESS:-127.0.0.1}:${ADMIN_PORT:-8080}:8080` ฝั่งโฮสต์จึงผูกกับที่อยู่เดียวที่ตั้งไว้ ไม่ใช่ทุก interface `./scripts/setup.sh` ปฏิเสธ `0.0.0.0`, IPv6 และ address สาธารณะ ไฟล์เว็บอยู่ที่ `WEB_DIST_DIR` (ค่าเริ่มต้น `/app/web`) ถ้าไม่มีไดเรกทอรีนี้ `/api/*` ยังทำงานและหน้าอื่นตอบ 404

**ไม่มี login ในแอป** ([ADR 0003](adr/0003-admin-app-protected-by-cloudflare-access-only.md)): หน้า Device setup แสดงรหัสผ่าน MQTT ของ `esp32` และใครเข้าถึงได้ก็เปลี่ยนค่าตั้งได้ ถ้าเปิดผ่าน Cloudflare Tunnel ต้องมี Cloudflare Access policy ที่ถูกต้องก่อน ไม่ใช้ `0.0.0.0` และไม่ forward port จาก router ไม่มี CORS (same-origin) ตัว API ตอบ `Cache-Control: no-store` ส่วน asset ที่มี hash ใน `/assets/` cache ได้ยาว

| API | ผล |
|---|---|
| `GET /api/status` | สถานะ server (mock หรือไม่), `device.state` (`online`/`offline`/`unknown`), ต่อ provider: `enabled`, snapshot ล่าสุดที่ MQTT ส่ง, `next_refresh_at`, `retry_after_seconds` |
| `GET`/`PUT /api/settings` | Server settings แบบแทนที่ทั้งก้อน: `providers.{claude,codex}.{enabled,refresh_interval_seconds}`, `stale_after_seconds`, `max_backoff_seconds`, `request_timeout_seconds` |
| `POST /api/refresh/{claude\|codex}` | `202` ถ้ารับคำสั่ง, `409 disabled`, `429 rate_limited` พร้อม `retry_after_seconds`, `404` |
| `GET`/`PUT /api/device` | `layout` (`cards`/`gauges`/`terminal`), `timezone` (POSIX TZ), `brightness` 10–100 |
| `GET /api/device-setup` | host, port, username, password ของ `esp32` (route เดียวที่ส่ง secret; `404` ถ้าไม่ได้ตั้ง `DEVICE_SETUP_HOST`) |

ค่าผิดกติกาตอบ `400 {"error":"invalid","fields":{"<path>":"<เหตุผล>"}}` และไม่เขียนไฟล์ ข้อผิดพลาดอื่นเป็น `{"error":"..."}` ขนาด body สูงสุด 16 KiB และ PUT/POST ที่มี body ต้องเป็น `application/json` ขอบเขตค่า: interval 60–3600, `stale_after_seconds` 60–86400, `max_backoff_seconds` 60–86400 และไม่น้อยกว่า interval ใดเลย, `request_timeout_seconds` 1–60

### ค่าตั้งและลำดับความสำคัญ

Server settings เก็บเป็น `settings.json` และค่า Device เป็น `device.json` ใน cache (`deploy/state/cache`, mock ใช้ `mock-cache`) เขียนแบบ atomic (temp + rename) โหมด `0600` ลำดับ: **ค่าในไฟล์ > env > ค่าเริ่มต้น** ถ้าไม่มีไฟล์ใช้ env/ค่าเริ่มต้น และไฟล์ที่เสียหรือผิดกติกาจะถูกข้าม (บันทึก warning ที่ไม่มีค่า) ไม่ทำให้ server ล้ม Env ที่อยู่นอกช่วงทำให้ server หยุดตอนเริ่มและบอกชื่อ env นั้น ลบไฟล์เพื่อกลับไปใช้ env ค่าเริ่มต้นของ `enabled` มาจาก `CLAUDE_PRIVATE_USAGE_ENABLED` และ `CODEX_ENABLED` (โหมด mock เปิด Claude เสมอจนกว่าไฟล์จะสั่งปิด) ไฟล์ทั้งสองอยู่ใน `STATE_DIR` จึงรวมอยู่ในการสำรอง state ของคุณ

**ย้ายจากรุ่นเดิม:** ตัวแปร interval ค่าเดียวที่ใช้ร่วมกันสองเจ้าถูกลบ ไม่มี alias ให้ลบบรรทัดนั้นออกจาก `deploy/.env` (ถ้ายังอยู่ Compose จะละเว้นเงียบ ๆ และ server ใช้ค่าเริ่มต้นใหม่) แล้วตั้ง `CLAUDE_POLL_INTERVAL_SECONDS` / `CODEX_POLL_INTERVAL_SECONDS` เพิ่ม `ADMIN_BIND_ADDRESS` และ `ADMIN_PORT` ถ้าต้องการเปลี่ยนค่าเริ่มต้น และรัน `./scripts/setup.sh` ซ้ำเพื่อตรวจ

### การปรับใช้ทันทีและ Retry-After

PUT settings มีผลโดยไม่ restart scheduler ของแต่ละ provider อ่านค่าสดและตื่นทันทีเมื่อค่าเปลี่ยน interval ที่สั้นลงมีผลเร็วที่สุดเท่าที่ทำได้ **แต่ไม่เร็วกว่า Retry-After หรือ backoff floor เดิม** ปิด provider แล้ว server ส่ง snapshot `unavailable`/`disabled` ทันที เปิดอีกครั้งจะเริ่มดึงใหม่ (Claude ยังต้องมี credentials; ถ้าไม่มีจะแสดง error code เดิมของ adapter) ปุ่ม Refresh now **ไม่ข้าม** Retry-After/backoff: ระหว่างนั้น API ตอบ `429` และ scheduler ตรวจซ้ำอีกชั้น หลัง error ที่ต้องแก้มือ (`reauth_required` เป็นต้น) scheduler หยุดดึงอัตโนมัติ `next_refresh_at` เป็น `null` และรอ login ใหม่แล้วกด Refresh now (หรือเปลี่ยนค่า enabled) แทนที่จะต้อง restart

### ตรวจสอบ

```sh
curl -s http://127.0.0.1:8080/api/status
curl -s -X PUT -H 'content-type: application/json' \
  -d '{"layout":"gauges","timezone":"ICT-7","brightness":60}' http://127.0.0.1:8080/api/device
./scripts/subscriber.sh 3 30   # device/config เป็น retained ใหม่ทันทีที่ PUT
```

`device/status` อ่านด้วยบัญชี `esp32` ไม่ได้ (เขียนอย่างเดียวตาม ACL) ดูสถานะอุปกรณ์จาก `device.state` ใน `/api/status` Log ของ server ไม่แสดง password นอกจากบรรทัด `Device setup form:` ตอนเริ่มที่มีอยู่เดิม

## คำสั่งดูสถานะและทดสอบ

subscriber เป็น read-only และรับ password จาก config file `0600` ไม่ใช้ `-P` บน argv:

```sh
# Keep subscribing until Ctrl-C.
./scripts/subscriber.sh
# Receive 3 retained messages, then exit; timeout is 30 seconds.
./scripts/subscriber.sh 3 30
# Inspect process status; logs contain safe enums only.
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock ps
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock logs --tail 30 quota-server-mock
# Offline test for secure setup behavior; no Docker or login.
python3 -m unittest discover -s deploy -p 'test_setup.py'
```

Broker restart และ retained persistence สำหรับ mock:

```sh
./scripts/subscriber.sh 3 30
# Stop publisher, leaving retained broker state in place.
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock stop quota-server-mock
docker compose --env-file deploy/.env -f deploy/compose.yaml restart mosquitto
./scripts/subscriber.sh 3 30
# Provider snapshots should still exist; server status is offline.
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock up -d quota-server-mock
./scripts/subscriber.sh 3 30
```

ทดสอบ LWT แบบไม่ graceful shutdown โดยรัน subscriber ค้างใน terminal แรก แล้วรัน terminal ที่สอง:

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock kill -s SIGKILL quota-server-mock
./scripts/subscriber.sh 3 30
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock up -d quota-server-mock
./scripts/subscriber.sh 3 30
```

หลัง SIGKILL broker ต้องเก็บ `{"state":"offline"}` แบบ retained หลังเริ่มอีกครั้งต้องเป็น `{"state":"online"}` และส่ง snapshot อีกครั้ง คำสั่ง `kill` ที่ Compose เรียกจะหยุดการ restart อัตโนมัติจนสั่ง `up` ใหม่ ถ้า consumer เห็น QoS 1 ซ้ำต้องไม่เพิ่มเครดิตหรือย้อนเวลา ยืนยัน persistence หลัง broker restart โดยไม่พึ่ง publisher ที่ยังทำงานอยู่

## หยุดและ restart

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live --profile mock down
# State survives because all persistent data uses host bind mounts.
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live up -d mosquitto quota-server
```

`down` ไม่ลบ bind state อย่าใช้ `rm -rf deploy/state` เป็นวิธีแก้ปัญหา restart broker ปกติบันทึก persistence ก่อนปิด การ hard kill broker อาจเสีย retained message ที่ยังไม่ถึงรอบ autosave จึงไม่ใช้เป็นขั้นตอนหยุดปกติ

## แก้ปัญหา

- **Permission denied:** ตรวจ `HOST_UID/HOST_GID` กับเจ้าของ state ทุกไดเรกทอรีต้อง `0700` และ credential/password file ต้อง `0600` อย่าเปลี่ยน server เป็น root หรือ chmod เปิดสาธารณะ
- **MQTT authentication/authorization:** รัน setup อีกครั้งเพื่อแปลง hash จากรหัสผ่านเดิม แล้ว restart broker ยืนยันว่า subscriber ใช้ `esp32` ไม่ใช่ `quota-server` อย่าเปิด anonymous หรือขยาย ACL เป็น `#`
- **Connection refused:** ตรวจว่า broker ทำงานและ `MQTT_BIND_ADDRESS` เป็น IP ที่โฮสต์มีอยู่จริง subscriber ภายใน Compose ใช้ชื่อ `mosquitto` ส่วนอุปกรณ์ใช้ LAN IP Firewall และ Wi-Fi isolation อาจปิดกั้นอุปกรณ์
- **reauth_required / auth_rejected:** ใช้ auth helper หยุด server แล้วเข้าสู่ระบบใหม่ ห้ามลบ credentials ก่อนสำรอง ห้ามยิง login หรือ refresh วน
- **access_denied / rate_limited:** รอ backoff/Retry-After ตรวจสิทธิ์สมาชิก ไม่ fallback ไป API หรือยิง inference
- **stale / unavailable:** ข้อมูลล่าสุดอาจยังแสดงอยู่ แต่เวลาเดิมไม่ใช่ข้อมูลสด ตรวจเวลาโฮสต์และ NTP ของอุปกรณ์ ข้อมูลที่หมดเวลา reset ต้องรอ provider ไม่เติม remaining เป็น 100% เอง
- **Claude disabled:** เป็นค่าเริ่มต้นเพื่อจำกัดความเสี่ยง private endpoint ดูขั้นตอนเปิดใน AUTH ไม่ใช่ข้อผิดพลาด Docker
- **process_unavailable:** ตรวจการ build image และ architecture Codex มี pin สำหรับ amd64/arm64 เท่านั้น ไม่มี socket app-server เปิดให้เครือข่าย
- **setup/auth lock ค้าง:** ตรวจว่าไม่มี process/helper ทำงานก่อนใช้ `rmdir deploy/.setup-lock` หรือ `rmdir deploy/.auth-lock` ไม่ลบล็อกขณะอีกคำสั่งทำงาน

ไม่แนบ raw upstream response, credential file, token, password, command environment หรือ auth transcript ใน issue/log แม้ปัญหาดูเหมือน auth ก็ส่งเพียง safe enum กับเวลาเท่านั้น

## อัปเกรด

สำรอง auth แบบเข้ารหัสตาม AUTH และหยุด live server ก่อนอัปเกรด Pin Codex, Claude และ SHA256 ทั้ง amd64/arm64 อยู่ใน `deploy/Dockerfile` ตรวจ release metadata ทางการและ flags ก่อนเปลี่ยนรุ่น อย่าใช้ `latest` ของ CLI อย่า download installer แล้วรันโดยไม่ตรวจ hash หลังแก้ pin ให้รัน offline tests และ mock smoke ก่อนบัญชีจริง:

```sh
python3 -m unittest discover -s deploy -p 'test_setup.py'
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile auth build auth
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock up -d --build mosquitto quota-server-mock
./scripts/subscriber.sh 3 30
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile mock stop quota-server-mock
./scripts/auth.sh codex status
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live up -d --build mosquitto quota-server
```

ถ้าอัปเกรด auth format แล้ว downgrade อาจอ่านข้อมูลไม่ได้ ให้ใช้ CLI ทางการเข้าสู่ระบบใหม่แทนแก้ JSON ด้วยมือ แหล่ง pin: [Codex 0.161.0](https://github.com/openai/codex/releases/tag/rust-v0.161.0), [Claude Code registry](https://registry.npmjs.org/@anthropic-ai/claude-code/2.1.293), [Mosquitto image](https://hub.docker.com/_/eclipse-mosquitto), [Rust official image manifest](https://github.com/docker-library/official-images/blob/master/library/rust)
