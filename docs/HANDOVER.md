# laphone 项目交接提示词（2026-08-03）

## 项目一句话
laphone：Windows 上 USB 有线手机投屏工具（类 scrcpy 但轻量、单 exe、托盘常驻、插线自动弹窗），手机端零安装（无 APK、无权限弹窗、重启即消失）。仓库 github.com/Ricky74192/laphone（public，Apache-2.0）。用户动机：摸鱼 + 开梯子时 PC 微信不及时，把手机屏投到电脑看。

## 当前状态（M0 已完成并提交）
零安装管线已跑通并四层验证：
`adb exec-out screenrecord --output-format=h264` → Annex-B NAL 拆分 → openh264 解码 → SDL2 IYUV 渲染（可缩放窗口+黑边等比）→ 点击注入（adb shell input tap，letterbox 坐标映射）
已有功能：可调大小窗口（初始=屏幕高 80%）、S 键软息屏（亮度0+stayon，退出自动恢复）、停滞自动唤醒（每 5s KEYCODE_WAKEUP，adb 不可达才退出）、screenrecord 断流自动重启、标题栏实时 fps。
架构决策：docs/ARCHITECTURE.md；实测记录：docs/DEVICE_TESTING.md（必读）。本地 D:\laphone，远端 main 2 个提交。

## 环境（重要）
- Windows + git-bash。Rust 工具链为 GNU（x86_64-pc-windows-gnu），**无 MSVC**，链接用 D:\DevTools\mingw64 的 gcc
- SDL2 用 bundled feature 源码编译（需 cmake + mingw32-make）；openh264 0.9.7（构建时自动下载 Cisco 预编译库）
- 原生 Windows 程序（ffmpeg/python/gcc）不认 MSYS 路径 /d/...，必须用 D:/...
- adb 在 PATH（D:\Android\Sdk\platform-tools）；ffmpeg/ffplay 8.1.1（gyan.dev）
- gh 便携版 C:\Users\13994\AppData\Local\Programs\gh\（每次会话需 export PATH）；已登录 Ricky74192
- 测试机：小米 13（fuxi，Android 16，HyperOS V816），序列号 a7e77894
- **USB 线易断连**：断连后 stayon 失效→屏幕睡→断流（app 已自动唤醒/恢复，但这是最常踩的坑）
- **中文输入法吞 ASCII 键**：按 S 前必须切英文模式（此前"按键没反应"的根因）

## 验证状态
PASS：编译、clippy -D warnings 零警告、离线仿真（deps/fakebin/fake_adb.c + ffmpeg 测试流；假 adb 的 stdout 必须 _setmode(_O_BINARY)，否则 CRLF 污染 H.264）、真机 16.4fps、息屏矩阵（真息屏 Dozing 断流 / 软息屏亮度0+stayon 续流 131帧@6s）。
待手动验证（需手机）：新版 letterbox 点击映射、S 键软息屏（英文输入法）、窗口缩放。
无测试套件；验证标准 = 编译 + clippy + 离线仿真 + 真机实测（ad-hoc，非套件绿灯）。

## 血泪坑
1. OpenH264 必须喂完整 NAL（Annex-B 起始码拆分、连起始码一起喂），否则丢 SPS/PPS → dsNoParamSets
2. SDL IYUV 纹理要精确 pitch；OpenH264 stride 有 padding（1080→1088），须重打包成紧致平面
3. screenrecord 息屏即断流 → 真息屏投屏只能靠 M1 临时 server（app_process + shell 权限 + wakelock + power-mode）
4. 软息屏 = 亮度 0 + stayon true；MIUI 最低亮度钳到 1（视觉全黑），screenrecord 续流
5. 杀进程别用 taskkill //IM（会连用户正在用的实例一起杀），用 PowerShell Stop-Process 或按 PID
6. sdl2 crate 0.37 不认 SDL2_DIR 环境变量，必须 bundled feature
7. 后台 bash 进程不继承前次调用的环境变量，跨调用传参用文件
8. gh 交互登录在 pty 卡 'Press Enter'；用 curl 设备码 API（client_id=178c6fc778ccc68e1d6a，scope 必须 "repo read:org gist workflow"）→ token 先落盘 → gh auth login --with-token

## 下一步
1. 插手机 → 跑 target/debug/laphone.exe，实测三个手动门槛（点击映射/S 键软息屏/窗口缩放），发现问题修复并提交
2. M1：adb push 临时 server（app_process 运行，shell 权限），MediaCodec 硬编 + InputManager 低延迟注入 + 真息屏（wakelock + power-mode），传输走 adb forward 抽象 socket
3. 差异化：触摸板多指手势→真多点触控注入、UHID 模式、托盘常驻+插线自动弹窗+单 exe 打包
