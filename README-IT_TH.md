# Document Atlas Web สำหรับ Windows Server x64

ชุดส่งมอบวันที่ 9 พฤศจิกายน 2026 ใช้ข้อมูลจาก Document Atlas ของบริษัท ผู้ใช้เปิดผ่าน URL HTTPS ที่ IT กำหนดด้วย Edge หรือ Chrome เลือกใช้ HTTPS ในชุดนี้หรือใช้ IIS ของบริษัท ทั้งสองวิธีส่งคำขอไปยัง Atlas ที่ `127.0.0.1:8765` บนเครื่องเซิร์ฟเวอร์

## ข้อมูลในชุด

- ดัชนี SQLite และเวกเตอร์เดิม 25,892 ช่วงข้อความ ค้นจาก RAG ได้ 1,920 เอกสาร มีโมเดล BGE-M3 รุ่นเดียวกับที่ใช้ทำดัชนี ดูรายละเอียดใน `data/snapshot.json`
- PDF ที่มีในเครื่อง 1,677 ไฟล์ รวมแคช 1 ไฟล์ ในจำนวนนี้ 1,675 ฉบับตรงกับดัชนีและเปิดในตัวอ่านได้จากชุดนี้ อีก 245 เอกสารที่ค้นด้วย RAG ได้ต้องใช้ Portal สำหรับฉบับ PDF หากพบว่า Portal เปลี่ยนฉบับ ระบบจะให้ IT อัปเดตดัชนีก่อนเปิด
- ตารางแผนก ผลคัดแยก 487 รายการ และคู่เอกสารคล้าย 5,757 รายการ บัญชีผู้อนุมัติบันทึกผลและส่งออกรายงาน CSV ได้
- ตัวอ่าน PDF ส่งภาพของหน้าที่เลือกจากเซิร์ฟเวอร์ จึงมีเฉพาะการอ่าน เปลี่ยนหน้า และซูมในเบราว์เซอร์
- เว็บชุดนี้ใช้ดัชนีที่ย้ายมาจาก Desktop งานสแกน Portal ทำดัชนีใหม่ และตรวจเอกสารใหม่ก่อนออกอยู่ในเครื่องดูแล Desktop ดูขั้นตอนดูแลข้อมูลด้านล่าง

## 1. ติดตั้งและทดลองบนเซิร์ฟเวอร์

1. ใช้ Windows Server x64 พร้อม [Node.js 24 x64](https://nodejs.org/en/download) ใน PATH และ [Microsoft Visual C++ Redistributable v14 x64](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist?view=msvc-170) เตรียม RAM 8 GB ขึ้นไปสำหรับทดลองใช้งานกับโมเดล BGE-M3 และพื้นที่ว่างอย่างน้อย 10 GB สำหรับ ZIP ข้อมูลที่แตก และสำรองฐานข้อมูล
2. แตก ZIP ด้วย `tar.exe` หรือเครื่องมือที่รองรับ ZIP64 ไปยังตำแหน่งสั้น เช่น `D:\Apps\DocumentAtlas` ไฟล์โมเดลหนึ่งไฟล์ใหญ่กว่า 2 GB เก็บโฟลเดอร์ชุดนี้ไว้นอก web root ของ IIS
3. ให้บัญชี Windows สำหรับรันงานมีสิทธิ์อ่านโฟลเดอร์แอปและเขียน `data` จำกัดสิทธิ์อ่าน `data` และ `accounts.json` ไว้ที่บัญชีรันงานกับ IT
4. เปิด PowerShell ในโฟลเดอร์ที่มี `Start-Atlas.ps1` สร้างบัญชีทดสอบและเริ่มเว็บ

```powershell
.\New-AtlasAccount.ps1 -Username 'iso.review' -Role reviewer
.\New-AtlasAccount.ps1 -Username 'iso.reader' -Role viewer
.\Start-Atlas.ps1
```

ระบบสร้างรหัสผ่านแบบสุ่มและแสดงครั้งเดียว ให้เก็บในระบบจัดการรหัสผ่านของบริษัทและสร้างบัญชีแยกรายคน หมุนรหัสผ่านด้วย `-Rotate` จากนั้นเริ่มเซิร์ฟเวอร์ใหม่เพื่ออ่านรายชื่อบัญชีล่าสุด

เปิด `http://127.0.0.1:8765` จากเบราว์เซอร์บนเครื่องเซิร์ฟเวอร์ และใช้บัญชีที่เพิ่งสร้าง `/healthz` ส่งสถานะบริการ HTTP 200 ให้ระบบตรวจสุขภาพได้ ส่วนหน้าและ API อื่นต้องยืนยันบัญชี

## 2. เปิดให้เครื่องอื่นเข้าใช้งานผ่าน HTTPS

### วิธี A ใช้ HTTPS ที่มากับชุดส่งมอบ

1. ให้ IT กำหนด IPv4 ของเซิร์ฟเวอร์และสร้าง DNS ภายในบริษัทให้ชี้มาที่ IP นั้น เช่น `iso.company.example` ใช้ใบรับรอง PFX พร้อม private key และ certificate chain ที่เครื่องผู้ใช้เชื่อถือ โดยชื่อ DNS ต้องตรงกับใบรับรอง
2. คัดลอก `server-config.lan.example.json` เป็น `server-config.lan.json` นำค่า `EnableChat`, `PortalUrl` และ `PortalCredentialFile` จากไฟล์เดิมมาด้วยหากตั้งไว้แล้ว ในส่วน `Lan` ตั้ง `Address` เป็น IPv4 ของเซิร์ฟเวอร์ `Hostname` เป็นชื่อ DNS จริง `Port` เป็นพอร์ต HTTPS เช่น 443 และ `PfxPath` กับ `PfxCredentialFile` เป็นพาธเต็มจริง ใช้ backslash สองตัวใน JSON ตามไฟล์ตัวอย่าง
3. เก็บ PFX ในโฟลเดอร์ควบคุมสิทธิ์ เช่น `D:\AtlasSecrets` สร้างไฟล์รหัสผ่าน PFX ด้วยบัญชี Windows เดียวกับที่รัน Atlas บนเซิร์ฟเวอร์ปลายทาง จำกัด ACL ให้เฉพาะบัญชีรันงานกับ IT

```powershell
Get-Credential -UserName 'atlas-tls' -Message 'Enter the PFX password' |
    Export-Clixml -LiteralPath 'D:\AtlasSecrets\atlas-tls.credential.xml'
```

4. หยุด Atlas รอบที่ทดสอบแบบ localhost แล้วรันคำสั่งต่อไปนี้จากโฟลเดอร์ชุดส่งมอบ เมื่อขึ้น `Document Atlas LAN is ready at https://...` ให้ใช้ URL นั้น

```powershell
.\Start-Atlas.ps1 -ConfigPath .\server-config.lan.json
```

5. ให้ IT เปิด inbound TCP เฉพาะพอร์ต HTTPS ที่ตั้งไว้และจำกัดต้นทางเป็น subnet ของผู้ใช้ในเครือข่ายบริษัท พอร์ตภายใน `8765` ใช้เฉพาะ loopback จากเครื่องผู้ใช้อีกเครื่องเปิด URL HTTPS แล้วลงชื่อเข้าใช้ด้วยบัญชี Atlas ที่สร้างไว้ ตรวจว่าเครื่องผู้ใช้เชื่อถือใบรับรองและลองค้นเอกสารกับเปิด PDF
6. เมื่อตรวจผ่าน ตั้ง Task Scheduler ให้รัน `Start-Atlas.ps1` ตอนเปิดเครื่องโดยส่ง `-ConfigPath` พร้อมพาธเต็มของ `server-config.lan.json` ใช้บัญชี Windows เดียวกับที่สร้างไฟล์รหัสผ่าน PFX และตั้งให้เริ่มใหม่หากโปรแกรมล้มเหลว เก็บสำเนาไฟล์ตั้งค่า LAN และต่ออายุ PFX ตามรอบบริษัท หลังเปลี่ยนใบรับรองให้เริ่ม Atlas ใหม่

ตัวอย่างคำสั่ง Firewall สำหรับผู้ดูแลระบบ เปลี่ยน IP และ CIDR ให้ตรงกับเครือข่ายจริง

```powershell
New-NetFirewallRule -DisplayName 'Document Atlas HTTPS' -Direction Inbound `
    -Action Allow -Protocol TCP -LocalAddress 10.0.10.20 -LocalPort 443 `
    -RemoteAddress 10.0.20.0/24 -Profile Domain,Private
```

### วิธี B ใช้ IIS หรือ reverse proxy ที่บริษัทมีอยู่

1. เริ่ม Atlas ด้วย `server-config.json` ตามข้อ 1 ให้ IIS หรือ reverse proxy ของบริษัทที่รันบนเครื่องเดียวกันส่งคำขอไป `http://127.0.0.1:8765` ใช้ชื่อ DNS และใบรับรอง TLS ของบริษัท ติดตั้งบน root ของ hostname เช่น `https://iso.company.example/`
2. ถ้าใช้ IIS URL Rewrite และ ARR ให้เปิด ARR Proxy แล้วใช้ `iis/web.config` ในโฟลเดอร์ว่างของ IIS site ให้ IIS ส่ง `Authorization` และ `WWW-Authenticate` ผ่านไปยัง Atlas เปิด Anonymous Authentication ที่ IIS site นี้เพื่อให้ Atlas ตรวจบัญชีเอง ตั้ง proxy timeout อย่างน้อย 180 วินาทีสำหรับถามตอบ ตั้งค่าการจำกัดการลองล็อกอินตามมาตรฐานของ IT [คู่มือ reverse proxy ของ Microsoft](https://learn.microsoft.com/en-us/iis/extensions/url-rewrite-module/reverse-proxy-with-url-rewrite-v2-and-application-request-routing)
3. เมื่อตรวจผ่าน ให้ตั้ง Windows Task Scheduler ให้รัน `powershell.exe -NoProfile -File "D:\Apps\DocumentAtlas\DocumentAtlas-Web-Windows-x64\Start-Atlas.ps1"` ตอนเปิดเครื่องด้วยบัญชีรันงานของ Atlas และเปิดการเริ่มใหม่เมื่อโปรแกรมล้มเหลว ใช้ตำแหน่งจริงที่แตกไฟล์ และใช้ execution policy ตามข้อกำหนด IT

สิทธิ์ `viewer` เปิดค้นหา ถามตอบ อ่าน PDF และดูผลคัดแยก สิทธิ์ `reviewer` เพิ่มการอนุมัติ ไม่อนุมัติ ยกเลิกผล และส่งออกรายงาน โดยบันทึกชื่อจากบัญชีที่ล็อกอิน จัดบัญชีให้เฉพาะบุคลากรที่ได้รับอนุญาตให้เห็นเอกสารทั้งหมดในพื้นที่ที่นำเข้าชุดนี้

## 3. เปิดถามตอบ GPT-6 Luna

ให้ IT ใช้บัญชี ChatGPT/Codex ที่บริษัทอนุญาตให้ส่งคำถามและช่วงข้อความอ้างอิงของเอกสารออกไปประมวลผล ใช้โควต้าของบัญชีดังกล่าว ZIP มี headless runtime และไฟล์ตั้งโมเดล GPT-6 Luna พร้อมใช้งาน ส่วนการติดตั้ง Codex และการล็อกอินทำด้วยบัญชี Windows เดียวกับที่ใช้รัน Atlas

```powershell
.\Install-AtlasCodex.ps1 -DeviceLogin
```

สคริปต์ติดตั้ง Codex CLI 0.166.0 ที่ทดสอบกับชุดนี้ผ่าน npm จากนั้นให้ผู้ดูแลทำขั้นตอนล็อกอินบนเว็บตามที่ CLI แสดง ตั้ง `"EnableChat": true` ในไฟล์ JSON ที่ใช้เริ่ม Atlas เช่น `server-config.lan.json` เมื่อรันด้วย `-ConfigPath` หรือ `server-config.json` เมื่อใช้ค่าเริ่มต้น แล้วเริ่ม Atlas ใหม่ ถ้าบัญชีไม่มีสิทธิ์โมเดลหรือโควต้าหมด ให้ IT ตรวจบัญชีนั้น ระบบใช้ GPT-6 Luna ตามที่กำหนดไว้

## 4. ตั้ง Portal สำหรับ PDF ที่อยู่นอกชุด

ไฟล์ที่รวมมาเปิดได้จากเซิร์ฟเวอร์โดยตรง หากต้องเปิด PDF ส่วนที่เหลือ ให้ IT ใช้บัญชี Portal ที่อ่านเอกสารตามสิทธิ์บริษัทและตั้ง `PortalUrl` ให้ตรงกับ Portal ของดัชนี ค่าดังกล่าวหาได้จากการตั้งค่า Portal ในโปรแกรม Desktop ต้นทาง เครื่องเซิร์ฟเวอร์ต้องเข้าถึง Portal ได้

```powershell
Get-Credential | Export-Clixml -LiteralPath 'D:\AtlasSecrets\portal.credential.xml'
```

รันด้วยบัญชี Windows ที่ใช้เริ่ม Atlas และตั้ง `PortalCredentialFile` เป็นตำแหน่งไฟล์นั้น DPAPI ผูกไฟล์กับเครื่องและบัญชี Windows ที่สร้าง ตั้ง ACL ของไฟล์ให้บัญชีรันงานอ่านได้ ระบบตรวจ SHA-256 ก่อนแสดงหน้า และเก็บสำเนาที่ตรงกับฉบับทำดัชนีไว้ที่ `data/rag-pdf-cache`

## 5. ตรวจรับและดูแลข้อมูล

1. ล็อกอินจากเครื่องผู้ใช้ที่ URL HTTPS ทดสอบบัญชีผู้อ่านและบัญชีผู้อนุมัติ
2. ค้น `STR-ES-W-41(2B)` แล้วเปิด **ตรวจข้อมูล RAG** เลือกหน้า 3 และกด **อ่าน PDF ต้นฉบับ** ลองเปลี่ยนหน้าและซูม
3. เลือก **ค้นจากเนื้อหา RAG** ค้น `การคัดแยกขยะและการจัดการของเสีย` ตรวจว่าผลมีชื่อและเลขหน้า การค้นเนื้อหาครั้งแรกต้องโหลดโมเดลเข้า RAM
4. เปิด **คัดแยก ISO** เลือกแผนกและดูเอกสารคล้าย ตรวจว่าผู้อ่านไม่มีปุ่มอนุมัติและผู้อนุมัติส่งออกรายงานได้
5. หลังตั้ง Codex ลองถาม `การคัดแยกขยะและของเสียในแผนก STR ต้องทำอย่างไร` แล้วเปิดหน้าเอกสารอ้างอิงจากคำตอบ
6. สำรองโฟลเดอร์ `data` และ `accounts.json` ตอนหยุดบริการ รวมถึง `server-config.json` การเปลี่ยนฐานข้อมูลควรสำรองผลอนุมัติจากเว็บและวางแผนย้ายผลกลับก่อนแทนที่ฐานข้อมูลด้วยข้อมูลใหม่จาก Desktop

`source/` เก็บซอร์สสำหรับ IT ดูแลต่อ หน้าเว็บสร้างด้วย `npm ci` และ `npm run build` ใน `source` แล้วนำ `dist/` ไปไว้ใน `public/` สร้างเซิร์ฟเวอร์ด้วย `cargo build --release --locked --manifest-path source/server/Cargo.toml` บน Windows ที่มี Rust MSVC และ C++ Build Tools
