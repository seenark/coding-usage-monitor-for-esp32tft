# การยืนยันตัวตนบัญชีสมาชิก

ระบบอ่านโควตาจากบัญชีสมาชิกหนึ่งบัญชีต่อผู้ให้บริการ ไม่ใช้ API key, Anthropic Console หรือการคิดค่าใช้จ่ายแบบ API ไม่เรียก inference ไม่ซื้อเครดิต และไม่รีเซ็ตโควตา ระหว่างพัฒนายังไม่ได้เข้าสู่ระบบหรือใช้บัญชีจริง

## พื้นที่แยกและผู้เขียนเพียงรายเดียว

รัน `scripts/setup.sh` ก่อนเริ่มใช้งาน พาธเริ่มต้นบนโฮสต์คือ `deploy/state/auth/` โดยทุกโฟลเดอร์เป็น `0700` และไฟล์ที่สร้างเป็น `0600` เจ้าของตรงกับ `HOST_UID` และ `HOST_GID` ใน `deploy/.env` ซึ่งต้องไม่เป็น root ภายในคอนเทนเนอร์ใช้:

- `HOME=/auth/home` เก็บสถานะ CLI เพิ่มเติมทั้งชุด ไม่เก็บเฉพาะ token
- `CODEX_HOME=/auth/codex` เป็น mount อ่านเขียนและบังคับ `cli_auth_credentials_store="file"` รวมถึง `forced_login_method="chatgpt"`
- `CLAUDE_CONFIG_DIR=/auth/claude` เป็น mount อ่านเขียนสำหรับข้อมูล CLI ของ Linux รวมถึง `.credentials.json`

อย่านำข้อมูล macOS Keychain หรือไฟล์ credentials ที่คัดเฉพาะบางฟิลด์มาแทนพื้นที่นี้ อย่าใช้ CLI อีกโปรเซสบน mount เดียวกันขณะที่เซิร์ฟเวอร์ทำงาน Codex CLI เป็นผู้ refresh credentials ของตนเอง ส่วนตัวเก็บ Claude อาจหมุน refresh token ดังนั้นการเขียนพร้อมกันอาจทำให้บัญชีต้องเข้าสู่ระบบใหม่

`scripts/auth.sh` ล็อกการยืนยันตัวตน หยุด live server ก่อนรัน CLI และเริ่ม live server อีกครั้งเฉพาะเมื่อเดิมทำงานอยู่และคำสั่งสำเร็จ ถ้าล้มเหลวหรือถูกยกเลิกจะปล่อยเซิร์ฟเวอร์ให้หยุดเพื่อหลีกเลี่ยงการเขียนซ้อน คำสั่ง status ก็ใช้ขั้นตอนเดียวกัน เพราะ CLI อาจ refresh ระหว่างตรวจสอบ

## Codex ผ่านบัญชี ChatGPT

จากราก repository:

```sh
./scripts/setup.sh
./scripts/auth.sh codex login
./scripts/auth.sh codex status
```

helper ใช้ Codex `0.161.0` และ device authorization ทางการ เปิด URL และกรอกรหัสชั่วคราวที่ CLI แสดงด้วยตนเองในเบราว์เซอร์ อย่าอัดวิดีโอ เก็บ transcript แชร์ URL/รหัส หรือส่ง output ไปยังระบบเก็บ logs เมื่อสำเร็จ helper ตรวจสถานะและแสดงเพียงผลสำเร็จ ไม่แสดง token หรือข้อมูลบัญชี

หาก device authorization ถูกปิด ให้เปิดสิทธิ์ device-code login ในการตั้งค่าความปลอดภัยของ ChatGPT หรือขอผู้ดูแล workspace ตาม [คู่มือยืนยันตัวตน Codex](https://developers.openai.com/codex/auth/) ห้ามเปลี่ยนไปใช้ API key

ทางเลือกทางการคือ browser login บนโฮสต์ที่ติดตั้ง Codex รุ่นเดียวกัน ใช้พื้นที่แยกชุดเดิมและหยุดเซิร์ฟเวอร์ก่อน ตัวอย่างนี้ใช้ `STATE_DIR=./state` หากปรับพาธให้แทนด้วยพาธจริง ห้ามเผยแพร่ callback port สู่ LAN หรืออินเทอร์เน็ต:

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live stop quota-server
CODEX_HOME="$PWD/deploy/state/auth/codex" RUST_LOG=off \
  codex -c 'cli_auth_credentials_store="file"' -c 'forced_login_method="chatgpt"' login
./scripts/auth.sh codex status
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live up -d quota-server
```

บนโฮสต์ระยะไกลใช้ SSH forwarding เฉพาะ loopback สำหรับ callback `1455` ตามเอกสารทางการ ไม่ใช้ public tunnel ไม่ใช้ browser callback ผ่าน port mapping ของ Docker เพราะ login listener ภายในคอนเทนเนอร์ฟัง loopback ของคอนเทนเนอร์เท่านั้น ถ้าคำสั่ง login ไม่สำเร็จ อย่ารันคำสั่งเริ่มเซิร์ฟเวอร์ต่อ

## Claude ผ่านสมาชิก Claude.ai

ตัวเก็บ private usage ของ Claude ปิดไว้โดยค่าเริ่มต้น การเข้าสู่ระบบไม่ได้เปิดตัวเก็บโดยอัตโนมัติ ใช้ helper ที่มี Claude Code `2.1.293`:

```sh
./scripts/auth.sh claude login
./scripts/auth.sh claude status
```

helper เลือก subscription login ของ Claude.ai ไม่ใช้ Anthropic Console และไม่รับ API key เปิด URL จาก CLI ในเบราว์เซอร์เครื่องของคุณ แล้วทำขั้นตอนยืนยันตัวตนใน terminal แบบโต้ตอบตามที่ CLI ขอ การแลกข้อมูลยืนยันตัวตนต้องทำผ่าน CLI ทางการเท่านั้น อย่าคัด token ลง command line หรือ env อย่าใช้ `claude setup-token` แทน credentials ของ CLI

หากเบราว์เซอร์ไม่เปิดอัตโนมัติ ให้คัดเฉพาะ URL ที่ CLI แสดงไปเปิดเอง และป้อนรหัสผ่าน prompt ทางการ หาก callback ใช้ไม่ได้ ให้ทำขั้นตอน headless/manual login ตาม [คู่มือ Authentication](https://code.claude.com/docs/en/authentication) และ [CLI reference](https://code.claude.com/docs/en/cli-reference) โดยยังอยู่ใน helper เดิมและ mount เดิม ไม่เข้าสู่ console ไม่ fallback ไป API ไม่คัด `.credentials.json` จาก macOS

เมื่อยืนยันสถานะสำเร็จและยอมรับความเสี่ยงแล้วจึงเปลี่ยน `CLAUDE_PRIVATE_USAGE_ENABLED=true` ใน `deploy/.env` แล้ว recreate live server:

```sh
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live up -d --force-recreate quota-server
```

Claude ใช้ endpoint แบบ private ที่ไม่ใช่ API สาธารณะ จึงอาจเปลี่ยนโครงสร้าง ถูกจำกัด หรือหยุดใช้งานได้ การยืนยันตัวตนสำเร็จไม่ได้รับประกันว่าสมาชิกหรือ workspace มีสิทธิ์อ่าน usage ตัวเก็บไม่ใช้ inference ทดสอบสิทธิ์ ไม่ยิงวนเมื่อ `401/403` และไม่เปลี่ยนไป API billing หากข้อมูลไม่พร้อมใช้งาน

## ยืนยันตัวตนใหม่

ใช้ `./scripts/auth.sh codex login` หรือ `./scripts/auth.sh claude login` เท่านั้น ไม่รัน `docker compose run auth` ขณะ live server ทำงาน ถ้า auth ล้มเหลวให้แก้ไขแล้วรัน helper ใหม่ หลังตรวจสถานะสำเร็จจึงเริ่ม live serverด้วยคำสั่ง `up -d quota-server` ข้างต้น หากล็อกค้างหลังไฟดับ ตรวจว่าไม่มี helper ทำงานด้วย `docker compose --env-file deploy/.env -f deploy/compose.yaml --profile auth ps -a` ก่อนลบ `deploy/.auth-lock` ด้วย `rmdir`

## สำรองแบบเข้ารหัส

ตัวอย่างต่อไปนี้สำหรับโฮสต์ Linux ที่มี GnuPG และ GNU tar ใช้ backup ของตนเองเท่านั้น ใส่ passphrase ผ่าน prompt ของ GnuPG ไม่ใส่ใน argv, env, shell history หรือไฟล์ repository สำรอง credentials ทั้งชุด ไม่เลือกเฉพาะ access token หยุด server และตรวจว่าไม่มี helper ก่อนสำรองเพื่อไม่ให้มีการหมุน token ระหว่างอ่าน:

```sh
umask 077
set -o pipefail
mkdir -p "$HOME/quota-auth-backups"
chmod 700 "$HOME/quota-auth-backups"
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live stop quota-server
tar -C deploy/state -cf - auth | \
  gpg --symmetric --cipher-algo AES256 --output "$HOME/quota-auth-backups/auth.tar.gpg"
chmod 600 "$HOME/quota-auth-backups/auth.tar.gpg"
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live up -d quota-server
```

ใช้ shell ที่เปิด `set -o pipefail` เพื่อให้ขั้นตอน tar หรือ gpg ล้มเหลวแล้วไม่ดำเนินต่อ ตรวจ exit status ก่อนเริ่มเซิร์ฟเวอร์ ห้ามเก็บ tar ที่ไม่เข้ารหัส สำรอง `deploy/.env` และ `deploy/state/secrets` แยกด้วยวิธีเข้ารหัสเดียวกันถ้าต้องการกู้ MQTT passwords และค่าตั้งทั้งหมด เก็บ passphrase แยกจาก backup ใน password manager อย่าสำรองผ่าน public tunnel

## กู้คืน

credentials เก่าอาจใช้ไม่ได้แล้วเมื่อ refresh token ถูกหมุนหลังสำรอง การกู้คืนไม่สามารถย้อน token ของผู้ให้บริการได้ ถ้ากู้แล้วเป็น `reauth_required` ให้เข้าสู่ระบบใหม่ ไม่ retry วน

หยุด live server และ helper ทั้งหมดก่อน กู้เฉพาะ backup ที่คุณสร้างและเชื่อถือ ตรวจว่าคอนเทนเนอร์ไม่มี writer ทำงาน หากต้องการเก็บชุดปัจจุบันให้ทำ encrypted backup ก่อน ห้าม extract archive จากบุคคลอื่น ใช้พาธ state จริงหากไม่ใช่ค่าเริ่มต้น:

```sh
umask 077
set -o pipefail
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live stop quota-server
gpg --decrypt "$HOME/quota-auth-backups/auth.tar.gpg" | \
  tar --extract --file - --no-same-owner --no-same-permissions -C deploy/state
find deploy/state/auth -type d -exec chmod 700 {} +
find deploy/state/auth -type f -exec chmod 600 {} +
# Replace these IDs with HOST_UID and HOST_GID from deploy/.env, not root.
sudo chown -R "$(id -u):$(id -g)" deploy/state/auth
./scripts/auth.sh codex status
# Run only if Claude is enabled for this installation.
./scripts/auth.sh claude status
docker compose --env-file deploy/.env -f deploy/compose.yaml --profile live up -d quota-server
```

ทำทีละคำสั่งและหยุดเมื่อคำสั่งล้มเหลว ถ้าเคยเปลี่ยนบัญชีหรือชื่อไฟล์ในพื้นที่ CLI อย่า extract ทับชุดที่ปะปน ให้ย้ายชุดเดิมไปที่พื้นที่ส่วนตัว `0700` หลังหยุด writer แล้วสร้างพื้นที่ auth ใหม่ก่อนกู้คืน ไม่ให้ backup หรือ credentials อยู่ใน Git, Docker build context, MQTT, firmware หรือ log aggregation
