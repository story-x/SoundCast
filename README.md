<div align="center">
<a href="https://github.com/story-x/SoundCast"><img src="assets/logo.png" width="120" alt="SoundCast"/></a>
</div>
<h1 align="center">SoundCast</h1>
<div align="center">

[![License](https://img.shields.io/github/license/story-x/SoundCast)](https://github.com/story-x/SoundCast/blob/main/LICENSE)
[![GitHub release](https://img.shields.io/github/v/release/story-x/SoundCast)](https://github.com/story-x/SoundCast/releases)
[![GitHub All Releases](https://img.shields.io/github/downloads/story-x/SoundCast/total)](https://github.com/story-x/SoundCast/releases)
[![GitHub stars](https://img.shields.io/github/stars/story-x/SoundCast)](https://github.com/story-x/SoundCast/stargazers)
[![GitHub forks](https://img.shields.io/github/forks/story-x/SoundCast)](https://github.com/story-x/SoundCast/forks)

<strong>一个极简、超轻量的 Windows 桌面工具，根据应用窗口当前所在的显示器，自动切换该应用的音频输出设备</strong>
</div>

## 功能特性

* 极度轻量，基于 Rust + Slint Fluent UI，采用“后台守护进程 + 按需弹出 UI 子进程”架构；无设置窗口时常驻内存约 **7~8 MB**、空闲 CPU 近 0%，关闭设置窗口后 UI 内存即刻释放
* 单应用独立音频路由：基于 Windows 10 (1803+) / Windows 11 底层 `AudioPolicyConfig` 接口实现进程级音频重定向，不影响系统全局默认输出设备
* 多进程应用精确识别：自动匹配同一可执行路径下的音频子进程，拖动 **Chrome、Edge、Discord、Electron** 等多进程应用跨屏时同样准确切换
* Windows 11 Fluent 现代界面：支持深浅色自适应主题、圆角卡片、运行进程带图标选择忽略列表与启动居中显示
* 开箱即用：支持系统托盘左键/右键打开设置、播放设备热插拔刷新、开机自启动（写入当前用户注册表 `Run` 项）以及绿色便携模式

## 安装与使用

可前往 [GitHub Releases](https://github.com/story-x/SoundCast/releases) 下载打包好的 `SoundCast.exe` 或压缩包后直接运行。

1. 运行 `SoundCast.exe`（首次启动尚未配置映射规则时会自动弹出设置界面；日常也可随时**左键点击系统托盘图标**或**右键菜单 -> 打开设置界面...**）。
2. 在设置界面中，为每个已连接的显示器选择对应的音频播放设备，可按需开启/关闭自动切换、调整防抖延迟（ms）、从运行中程序下拉列表添加/移除忽略进程或开启开机自启，最后点击 **保存并应用** 即可热加载生效。
3. 点击 **隐藏到托盘** 或直接关闭设置窗口后，UI 子进程退出释放全部界面内存，后台守护进程继续静默运行。将任意应用窗口拖动或通过快捷键（`Win + Shift + ←/→`）移动到目标显示器，该应用音频会自动切换至对应设备。

### 配置文件

* 默认保存路径：`%APPDATA%\SoundCast\config.toml`
* **绿色便携模式**：只需在 `SoundCast.exe` 同级目录下新建一个空文件 `config.toml`，程序即优先读写同目录配置

### 已知限制

* 部分使用独立音频引擎的第三方播放器不会即时响应系统级“进程默认音频端点”动态变更，策略写入后可能需要暂停/继续或切歌重建音频流才会生效。
* Chromium 系浏览器（如 Chrome、Edge）同一实例下的多个窗口共享同一个音频子进程会话，跨屏时会按进程整体切换到当前激活或移动窗口所在显示器的播放设备，无法按单个标签页独立路由。

## 构建客户端

### 运行环境要求

* Windows 10 (1803+) / Windows 11
* Rust 工具链（推荐 `stable-x86_64-pc-windows-msvc`）与 Visual Studio C++ 生成工具

### 拉取代码

```bash
git clone git@github.com:story-x/SoundCast.git --depth=1
cd SoundCast
```

### 本地 Debug 启动与一键打包

* **本地 Debug 快速启动（后台守护 + 弹出设置界面）**：
  直接双击 `run-debug.bat`，或在终端执行：
  ```powershell
  .\run-debug.ps1
  ```
  仅预览设置界面可附加 `-SettingsOnly` 参数：
  ```powershell
  .\run-debug.bat -SettingsOnly
  ```
* **本地 Release 一键打包（输出 `SoundCast.exe`、`.zip` 绿色压缩包与 `.msi` 安装包到 `dist/`）**：
  直接双击 `package-release.bat`，或在终端执行：
  ```powershell
  .\package-release.ps1
  ```
  如需本地打 `.msi` 安装包，需预先安装 WiX Toolset v5 CLI 与 UI 扩展（`dotnet tool install --global wix --version 5.0.2` 及 `wix extension add -g WixToolset.UI.wixext/5.0.2`），未安装时也可加 `-SkipMsi` 仅生成 `.zip`。

### 手动编译

```powershell
cargo +stable-x86_64-pc-windows-msvc build --release --target x86_64-pc-windows-msvc
```

编译产物位于 `target\x86_64-pc-windows-msvc\release\SoundCast.exe`。

### GitHub Actions 自动构建

仓库内置 `.github/workflows/release.yml`：

* 推送代码到 `main` 分支或手动点击 **Actions -> Run workflow** 时，会自动编译并打包 `SoundCast_<branch>_<时间>.zip` 与 `.msi` 安装包作为 Workflow Artifacts（保留 5 天）。
* 推送任意 `v*` 标签（例如 `git tag v0.1.0 && git push origin v0.1.0`）时，会自动编译生成 `SoundCast_v0.1.0.zip` 与 `SoundCast_v0.1.0.msi` 并发布到 **GitHub Releases**。

## 参考与致谢

本项目在底层 Windows 音频策略路由与跨屏窗口监听思路上参考了以下优秀开源项目（在此致谢）：

* **[ScreenSoundSwitch](https://github.com/Lingwuxin/ScreenSoundSwitch)**：参考了其按显示器自动路由应用音频的设计思路，以及 `SoundSwitch.Audio.Manager` 中 `Windows.Media.Internal.AudioPolicyConfig` 接口 GUID、vtable 槽位、`\\?\SWD#MMDEVAPI#` 设备 ID 拼接格式、多进程可执行路径匹配及窗口位置监听机制。
* **[SoundSwitch](https://github.com/Belphemur/SoundSwitch)**：`ScreenSoundSwitch` 所集成的 `SoundSwitch.Audio.Manager` 音频端点切换与窗口监控组件源自该项目。
* **[EarTrumpet](https://github.com/File-New-Project/EarTrumpet)**：最早在 Windows 上整理并验证 `SetPersistedDefaultAudioEndpoint` 等单应用音频路由机制的先驱项目。
* **[Slint](https://slint.dev/)**：本项目的 Fluent 现代设置界面基于 Slint UI 工具包构建。

## 开源协议

本项目基于 [MIT License](LICENSE) 开源协议发布。
