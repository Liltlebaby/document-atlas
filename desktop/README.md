# Document Atlas Desktop สำหรับผู้ดูแลข้อมูล

ตัวติดตั้ง `Document Atlas_0.7.0_x64-setup.exe` ใช้กับเครื่อง Windows ที่ผู้ดูแลใช้สแกน Portal จัดทำดัชนี RAG และเตรียมข้อมูลสำหรับเว็บ ตัวติดตั้งมีโปรแกรมและส่วนประมวลผลถามตอบของ Desktop ฐานข้อมูลที่ย้ายมาอยู่ในโฟลเดอร์ `data` ของชุดเว็บ

การเปิดเว็บให้ผู้ใช้ผ่าน URL ของบริษัทเริ่มที่ `README-IT.md` ในโฟลเดอร์หลัก คู่มือสำหรับผู้ใช้และ IT อยู่ใน `docs`

## สร้างตัวติดตั้งจากซอร์ส

ใช้ Windows x64 พร้อม Node.js, Rust แบบ MSVC, C++ Build Tools และ WebView2 เปิด PowerShell ในโฟลเดอร์ `desktop/source` แล้วรัน

```powershell
npm ci
npm ci --prefix qa-runtime --omit=dev
npm run build:windows
```

ไฟล์ติดตั้งอยู่ที่ `src-tauri/target/release/bundle/nsis` บัญชี Portal และ Codex ตั้งแยกบนเครื่องที่ใช้งานตามสิทธิ์ของบริษัท
