# 代办：桌面悬浮待办

“代办”是一款本地优先的 Windows 桌面悬浮待办：想到事情时快速记下，按自己需要的顺序处理，并保留完成历史作为周报和回顾依据。

当前版本为 `0.1.0`，属于可试用的早期版本。Windows x64 NSIS 安装器和自动更新链路已经实现，本机已成功生成安装器与更新签名；首个 GitHub Release 尚未创建，因此当前不能把它视为已经完成线上发布。宠物、番茄钟、AI 和日历均不在本阶段范围内。

## 功能

- 快速记录待办，并在展开面板、当前任务胶囊和系统托盘之间切换。
- 每条待办都可直接勾选完成，也可软删除；删除会移出待办队列，但保留事件历史且不产生奖励。
- 双击未完成待办标题，或聚焦后按 `Enter` / `F2`，可在原位置修改标题；`Enter` 或失焦保存，`Esc` 取消。
- 截止日期默认不设置，可在编辑待办时额外选择或清除。列表只在有期限时显示“今天”“明天”“M/D”“YYYY/M/D”或“逾期 N 天”等紧凑标签。
- 通过拖动柄或 `Alt+↑/↓` 调整完整待办顺序。
- 在待办页和完成记录页按 `Ctrl+F`，或从更多菜单进入当前页面搜索。搜索只按标题实时筛选并高亮；搜索期间仍可完成、修改、删除或撤销，但禁止重排过滤后的局部列表。
- 独立查看完成记录和完成时间，并通过 `↶` 撤销完成；撤销后的任务回到待办队尾。
- 长列表只在列表内部滚动，录入区、完成记录标题和底部入口保持固定。
- 正常模式启动后会检查更新，并每 24 小时再次检查；也可从更多菜单手动检查。发现新版本后，由用户明确点击安装。

## 技术架构

- 桌面壳：Tauri 2。
- 前端：原生 HTML、CSS、JavaScript 模块（ES modules），不依赖前端框架和构建器。
- 核心与应用服务：Rust，负责任务状态、队列、事件、奖励、幂等和窗口运行模式。
- 持久化：SQLite，本地任务投影、事件、奖励和命令回执在同一事务中提交。
- 可靠写入：前端通过操作箱保存稳定操作 ID，再经 `LedgerSession → TauriGateway → Tauri 命令 → TaskService` 提交意图；前端只渲染后端快照，不复制领域状态机。
- 更新适配：`UpdateController → TauriGateway → 项目自有更新命令 → AppUpdateService → Tauri updater`。前端只负责检查时机和交互，Rust 负责运行模式校验、待安装版本与单次安装锁。

代码依赖方向和可靠性不变式见[《代码架构与模块边界 v0.1》](./docs/代码架构与模块边界-v0.1.md)，事件与任务状态说明见[《任务状态与事件账本 v0.1》](./docs/任务状态与事件账本-v0.1.md)，发布边界与操作流程见[《Windows 发布与自动更新 v0.1》](./docs/Windows发布与自动更新-v0.1.md)。

## 运行与构建

### Windows 前置条件

- Windows 10 或 Windows 11。
- Node.js 与 npm，建议使用当前维护中的 LTS 版本。
- 通过 `rustup` 安装的 Rust stable 工具链。
- Visual Studio 2022 Build Tools，并安装“使用 C++ 的桌面开发”工作负载、MSVC 工具集和 Windows SDK。
- Microsoft Edge WebView2 Runtime。

以下命令均从仓库根目录运行。Windows PowerShell 中使用 `npm.cmd`，避免执行策略拦截 `npm.ps1`。

安装锁定依赖：

```powershell
npm.cmd ci
```

启动开发版：

```powershell
npm.cmd run desktop:dev
```

运行统一测试门禁：

```powershell
npm.cmd run desktop:check
```

该命令会先运行前端 Node 测试，再运行 Rust 测试。需要进一步验证本地账本或桌面联合链路时，可运行：

```powershell
npm.cmd run ledger:smoke
npm.cmd run desktop:smoke
```

构建不带安装器的 release EXE：

```powershell
npm.cmd run desktop:build
```

产物位于 `src-tauri/target/release/zuoban-desktop-spike.exe`。用户可见产品名与 Windows 文件元数据为“代办”；内部 identifier、EXE 名和数据库文件名继续保留 `zuoban` 兼容名称，避免破坏已有本地数据。

构建 Windows x64 NSIS 安装器与更新签名：

```powershell
npm.cmd run desktop:bundle:windows
```

安装器与 `.sig` 更新签名位于 `src-tauri/target/release/bundle/nsis/`。本机已经验证两类产物可以成功生成；首个 GitHub Release 与线上 `latest.json` 尚未创建，完整线上更新闭环仍待首次发布后验证。

## 安装、发布与更新

- 首次安装：首个 Release 发布后，用户需要从 GitHub Releases 手动下载 Windows x64 `setup.exe` 并安装；安装范围为当前用户。
- 后续更新：正常模式启动后立即静默检查一次，此后每 24 小时检查；更多菜单提供“检查更新”。发现新版本后，用户点击“安装更新”才会下载、安装并重启应用。
- 冒烟隔离：smoke 前端不会启动更新控制器，Rust 更新服务也会在访问网络前拒绝请求，避免测试触碰正式更新源。
- 自动发布：`.github/workflows/release.yml` 支持从 `main` 手动触发和 `v*` 标签触发；`tauri-apps/tauri-action@v1` 先生成草稿 Release、安装器、更新签名和 `latest.json`，工作流用客户端公钥验签并核对元数据后才自动发布。
- 发布密钥：仓库只约定 GitHub Actions Secrets `TAURI_SIGNING_PRIVATE_KEY` 与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；私钥和密码不得提交到仓库，应在受控的加密离线位置保留恢复备份。
- Windows Authenticode 代码签名暂未配置。Tauri 的 `.sig` 用于校验自动更新产物，不能替代 Authenticode；首次运行安装器时可能出现 Microsoft Defender SmartScreen 提示。

## 数据与隐私

- 待办、完成历史、事件和奖励账本保存在本机 Tauri 应用数据目录的 SQLite 数据库中。
- 核心待办流程可离线运行，不接入云同步、AI 服务或外部日历，也不会把截止日期发送到外部系统；只有正常模式的更新检查会访问 GitHub Releases。
- 写命令使用幂等回执和可靠操作箱；结果未知时会保留原操作并锁住后续写入，不会静默重复执行。
- 数据库结构当前为 v4；旧结构会按版本迁移，并在需要的迁移前生成一致性备份。
- 面向用户的删除是可追溯软删除，不会物理清除历史。永久清理、通用自动备份、手动导出和跨设备恢复尚未实现。

## 已知限制

- 当前只按 Windows 桌面环境开发和验证，不承诺 macOS 或 Linux 可用。
- 当前安装器只面向 Windows x64；首个 GitHub Release 尚未创建，首次下载与线上自动更新闭环仍待验证。
- Windows Authenticode 暂未配置，安装器可能触发 SmartScreen 提示。
- 多显示器、所有 Windows 缩放比例以及任务栏位于四个方向的组合仍需更多实机验证。
- 截止日期只用于本地展示，不会自动排序、隐藏任务、提醒、发送通知、同步日历或产生奖惩。
- 当前不提供有道待办同步或历史迁移；产品仍处于试用阶段，尚未达到完整替代有道待办的发布标准。
- 宠物成长、番茄钟、AI、日历联动、周报生成、阻塞/恢复工作流、通用备份和导出均为后续方向。

## 设计与文档

极简原生风是当前实现和验收基线。展开面板只保留快速记录、统一待办列表和完成记录入口；低频技术操作收进右上角更多菜单。

### 当前主方案

- [极简原生主方案完整流程原型](./prototype/native/index.html)
- [桌面悬浮窗口技术实验 v0.1](./docs/桌面悬浮窗口技术实验-v0.1.md)
- [本地事件历史、金币账本与异常恢复技术实验 v0.1](./docs/本地事件历史、金币账本与异常恢复技术实验-v0.1.md)
- [界面与交互设计说明 v0.2](./docs/界面与交互设计说明-v0.2.md)
- [产品设计方案 v0.1](./docs/产品设计方案-v0.1.md)
- [产品路线图 v0.1](./docs/产品路线图-v0.1.md)
- [Windows 发布与自动更新 v0.1](./docs/Windows发布与自动更新-v0.1.md)

### 保留候选设计

三种视觉设计全部保留，但第一版不并行开发三套完整主题：

- [极简原生风格研究稿](./prototype/themes/native.html)——当前主方案。
- [深色透明风](./prototype/themes/glass.html)——后续候选皮肤。
- [像素桌宠风](./prototype/themes/pixel.html)——后续候选皮肤。
- [三种风格统一比较入口](./prototype/themes/index.html)
- [视觉方向决策 v0.3](./docs/视觉方向决策-v0.3.md)
- [三种视觉风格对比 v0.2](./docs/视觉风格对比-v0.2.md)

深色透明和像素桌宠只复用同一套组件、协议和业务逻辑，不会各自复制一套任务实现。

### 历史探索

- [第一轮暖色手账风原型](./prototype/index.html)
- [第一轮界面设计说明 v0.1](./docs/界面设计说明-v0.1.md)

暖色手账风只作为历史探索保留，不是当前实现基线。

### 浏览器预览原型

主方案网页原型用于保存完整业务设想，不能替代真实 Tauri 桌面应用。默认可直接打开 `prototype/native/index.html`；若浏览器限制本地脚本，可在仓库根目录运行：

```powershell
python -m http.server 4173 --bind 127.0.0.1 --directory .\prototype
```

然后访问 `http://127.0.0.1:4173/native/`。

## 许可证

本项目使用 [MIT License](./LICENSE) 开源。
