# ScreenCrosshair

在屏幕中心显示准星的全屏透明覆盖层（Windows）。准星**不拦截鼠标点击**，可以一直挂着用。

- 14 个内置样式，颜色 / 透明度 / 缩放都能实时调
- 支持用 JSON 图元自己画准星（线段、圆点、圆环、圆弧、矩形、多边形、组合）
- 四种改配置的方式：**热键**、**托盘菜单**、**内置设置界面**、**手改 `config.json`（自动热重载）**
- 19 个全局热键，**能改键、能一键全关**（关掉后一个组合键都不占用）
- 多显示器可选、高 DPI 自适应、开机自启

## 安装

### 方式一：安装包（推荐）

从 [Releases](https://github.com/Aaze-wu/ScreenCrosshair/releases) 下载
`ScreenCrosshair-<版本>-setup.exe` 双击安装即可。

- 默认**按当前用户**安装到 `%LOCALAPPDATA%\Programs\ScreenCrosshair`，不需要管理员；
  安装向导里可以切成「所有用户」（那份装到 `Program Files`）。
- 安装时如果程序正在运行，安装包会**请它自己退出**（调 `--quit`），不会弹
  「请先关闭程序」——这个程序没有任务栏窗口，那种提示等于死路。
- 自启项落在「启动」文件夹，不写注册表 `Run`（原因见下）。卸载时会一并清掉。
- 卸载**不会**删你的设置（`%APPDATA%\ScreenCrosshair`），想连配置一起删就手动删这个目录。

自己打包：

```powershell
powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
```

它会先 `cargo build --release` 再调 Inno Setup 的 `ISCC.exe`，产物在 `dist\`。
需要 [Inno Setup](https://jrsoftware.org/isinfo.php) 6 或 7。版本号只写在 `Cargo.toml`
里：`build.rs` 把它写进 exe 的 VERSIONINFO，`installer\ScreenCrosshair.iss` 再读回来，
所以不会出现「exe 是 0.1.0、安装包写着 0.2.0」。

### 方式二：直接拿 exe

```powershell
cargo build --release
```

产物在 `target\release\screencrosshair.exe`，单文件、无运行时依赖，直接复制走就能用。

想开机自启就不用自己建快捷方式：托盘菜单或设置里勾「开机自启」，程序会在
`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup` 放一个 `ScreenCrosshair.lnk`
（只动当前用户目录，不需要管理员）。

> 这里**故意不用** `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`：联想电脑管家这类
> 「开机启动优化」会在写入后 1~2 秒把该项搬进 `Run\LenovoDisabled` 并加 `rem|` 前缀标记为禁用，
> 表现就是「勾上又被弹回未勾选、下次开机不运行」。启动文件夹不受这类工具处理。
> 老版本写过的注册表值会在下次开启/关闭自启时顺手清掉。

### 只允许一个实例

程序用命名互斥体（`Local\ScreenCrosshair.SingleInstance`）保证只跑一份：
重复双击不会再开第二个覆盖层（那会导致两套全局热键抢注册、两个托盘图标、两层叠加的准星）。
而且第二次双击**不会静默退出**——它会给已有实例投一条 `WM_APP` 消息，把设置界面弹出来，
因为你双击图标的意图显然是「把程序叫出来」。

`--validate` / `--print` 这些命令行模式不受限制，可以随便并行跑。

## 命令行

| 参数 | 说明 |
| --- | --- |
| 无参数 | 正常运行（覆盖层 + 托盘图标） |
| `--config <路径>` | 指定配置文件位置 |
| `--validate` | 校验配置文件并打印结果 |
| `--print` | 打印「实际生效」的准星描述（JSON） |
| `--list-presets` | 列出所有预设名 |
| `--preview <目录>` | 把每个预设渲染成 PNG（调试用） |
| `--quit` | 让正在运行的实例退出（安装包用；没在运行也算成功） |
| `--help` / `--version` | 帮助 / 版本 |

## 热键

全部全局有效（游戏在前台也管用）。下表是**默认值**。

| 热键 | 动作 |
| --- | --- |
| `Ctrl+Alt+→` | 下一个预设 |
| `Ctrl+Alt+←` | 上一个预设 |
| `Ctrl+Alt+H` | 显示 / 隐藏准星 |
| `Ctrl+Alt+↑` | 提高不透明度 |
| `Ctrl+Alt+↓` | 降低不透明度 |
| `Ctrl+Alt+Shift+↑` | 放大 |
| `Ctrl+Alt+Shift+↓` | 缩小 |
| `Ctrl+Alt+O` | 打开设置界面 |
| `Ctrl+Alt+R` | 重新载入配置 |
| `Ctrl+Alt+Q` | 退出 |
| `Ctrl+Shift+A/D/W/S` | 准星左 / 右 / 上 / 下移 1 像素 |
| `Ctrl+Alt+Shift+A/D/W/S` | 同上，一次 10 像素 |
| `Ctrl+Alt+0` | 准星回到屏幕中心 |

### 改键与总开关

- **改键**：设置界面「全局热键」区，点某一行的「改键」，然后直接按下想要的组合键（`Esc` 取消）。
  单个动作可以「恢复默认」，底部还有「全部恢复默认」。改完立刻生效并写进 `config.json` 的 `hotkeys`。
- **暂挂**：捕获新键位期间，程序会**临时注销全部热键**。必须这样——被 `RegisterHotKey` 占用的组合
  不会再进任何窗口的键盘队列，否则你连「正在用的那个键」都按不出来。
- **总开关**：「启用全局热键」勾选框，或托盘右键菜单里同名项。关掉后所有组合键都不响应（改键面板也会变灰）。

### 注册失败怎么办

`Ctrl+Alt+字母` 是很热门的组合，常被输入法、显卡驱动、录屏软件先占掉；Windows 同一个组合
只肯给第一个注册的进程。**注册失败的项会在设置界面顶栏列出来**，其余热键照常工作。

默认的 1 像素微调之所以用 `Ctrl+Shift+A/D/W/S` 而不是 `Ctrl+Alt+…`，就是因为后者在实测机器上四个全被占了。
换成 `Ctrl+Shift+字母` 一般都能拿到。

## 调整准星位置

准星默认画在**屏幕几何中心**。需要挪一点（比如游戏自带的准星偏上）有三个入口，效果完全一致：

| 入口 | 操作 |
| --- | --- |
| 热键 | `Ctrl+Shift+A/D/W/S` 微调 1 像素，`Ctrl+Alt+Shift+A/D/W/S` 一次 10 像素，`Ctrl+Alt+0` 回中心 |
| 托盘菜单 | 右键托盘图标 → 「位置」→ 按 1 / 10 像素四向移动，或「回到屏幕中心」 |
| 设置界面 | 参数区「位置」的 X / Y 输入框，或「回到中心」按钮 |

三个入口改的都是配置里的 `overrides.offset`，会写进配置文件，重启后依然有效（`Ctrl+Alt+R`
重新载入配置会把它读回来）。单位是「100% DPI 下的像素」：在 150% 缩放的显示器上挪 4 格
实际移动 6 个物理像素——和准星尺寸的缩放规则一致，不会出现「大小对得上、位置对不上」。

## 内置预设

`dot` 圆点 · `dot_small` 小圆点 · `cross` 十字 · `cross_thin` 细十字 · `cross_dot` 十字+点 ·
`circle` 圆环 · `circle_dot` 圆环+点 · `cross_circle` 十字+圆环 · `t_shape` T 形 ·
`chevron` V 形 · `triangle` 三角箭头 · `brackets` 四角括号 · `square` 方框 · `x_cross` 斜十字

## 配置文件

默认位置 `%APPDATA%\ScreenCrosshair\config.json`。**改完保存即刻生效**（约 0.4 秒内热重载），
不用重启。文件缺失时会自动生成一份。

```json
{
  "version": 1,
  "active": "cross_dot",
  "visible": true,
  "monitor": "primary",
  "overrides": {
    "color": "#FF3B30",
    "opacity": 0.7,
    "scale": 2.0
  },
  "custom_presets": {}
}
```

### 顶层字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `version` | 数字 | 配置格式版本，当前固定 `1` |
| `active` | 字符串 | 当前预设名：内置名、或 `custom_presets` 里的名字 |
| `visible` | 布尔 | 准星是否显示 |
| `monitor` | `"primary"` / `1` / `"1"` | 画在哪块屏幕。`primary` = 主屏；数字是 **0 基**下标 |
| `overrides` | 对象 | 对当前预设的临时覆盖，见下 |
| `custom_presets` | 对象 | 自定义预设，键是名字、值是完整的准星描述 |

### `overrides`

只影响显示，不改预设本身，所以换个预设这些值仍然生效。全部可省略。

| 字段 | 类型 | 范围 |
| --- | --- | --- |
| `color` | 字符串 | `#RRGGBB` 或 `#RRGGBBAA` |
| `opacity` | 数字 | `0.1` – `1.0` |
| `scale` | 数字 | `0.3` – `6.0` |
| `rotation_deg` | 数字 | 任意角度，顺时针 |
| `offset` | `[x, y]` | 相对屏幕中心的偏移，**单位：100% DPI 像素**，`x` 向右、`y` 向下为正，上限 ±10000 |
| `outline` | 布尔 | 是否画描边 |
| `outline_color` | 字符串 | 描边颜色 |
| `outline_width` | 数字 | 描边宽度（像素） |

## 自定义准星

`custom_presets` 里放一个完整的准星描述。**坐标原点在屏幕中心**，单位是 100% DPI 下的像素，
实际显示时会再乘 `scale` 和显示器 DPI 缩放。

```json
{
  "custom_presets": {
    "my_cross": {
      "color": "#00FF88",
      "opacity": 0.9,
      "scale": 1.0,
      "rotation_deg": 0,
      "offset": [0, 0],
      "outline": { "enabled": true, "color": "#000000", "opacity": 0.55, "width": 1.0 },
      "shapes": [
        { "kind": "line", "from": [-20, 0], "to": [-4, 0], "width": 2 },
        { "kind": "line", "from": [4, 0], "to": [20, 0], "width": 2 },
        { "kind": "dot", "radius": 2 }
      ]
    }
  }
}
```

存了以后把 `"active"` 改成 `"my_cross"` 就生效了；设置界面里也能直接编辑脚本、校验、另存。

### 七种图元

| `kind` | 必填 | 可选 |
| --- | --- | --- |
| `line` | `from` `[x,y]`、`to` `[x,y]` | `width`（默认 2）、`cap` = `butt`/`round`/`square` |
| `dot` | `radius` | `center`（默认 `[0,0]`） |
| `circle` | `radius` | `center`、`filled`（默认 `false`）、`width` |
| `arc` | `radius`、`sweep_deg` | `center`、`start_deg`（0° 指向右，顺时针为正）、`width`、`cap` |
| `rect` | `size` `[宽,高]` | `center`、`corner_radius`、`filled`、`width`、`rotation_deg` |
| `polygon` | `points` `[[x,y], …]` | `closed`（默认 `true`）、`filled`、`width` |
| `group` | `shapes`（图元数组） | `rotation_deg`、`translate`、`scale` |

`group` 用来给一组图元整体加变换，省得手算每个点的坐标：

```json
{
  "kind": "group",
  "rotation_deg": 45,
  "shapes": [
    { "kind": "line", "from": [-16, 0], "to": [-5, 0], "width": 2 },
    { "kind": "line", "from": [5, 0], "to": [16, 0], "width": 2 }
  ]
}
```

`polygon` 画三角形（`filled: true`）：

```json
{
  "kind": "polygon",
  "filled": true,
  "points": [[0, -9], [8, 7], [-8, 7]]
}
```

## 设置界面

托盘菜单或 `Ctrl+Alt+O` 打开。左边选预设，中间是实时预览和参数（颜色、不透明度、尺寸、
旋转、**位置偏移**），底部是可编辑的 JSON 脚本区。预览固定画在正中——它反映「长什么样」，
不反映「摆在哪」。

两个交互细节：

- **中间参数区可滚动**，窗口再矮也能翻到底部的「全局热键」。
- **底部脚本面板能调高**：拖面板头顶那根横杠（高 10px）上下拽；**双击横杠复位到默认高度**。
  高度会被限制在「至少 96px、最多窗口高的 75%」，不会把中间的预览区挤没。

关掉窗口只是**隐藏**，再打开是同一个窗口，配置状态不会丢。

## 关于 / 更新检测

设置界面底部的「关于 / 更新检测」可以显示版本、构建平台、生成日期，并手动查新版。

**只提示，不下载。** 点「检查更新」时程序用 WinHTTP 接一次 GitHub Releases 的
`latest` 接口，比对版本号；有新版的就显示绿色提示 + 「打开发布页」按钮，把链接
交给系统默认浏览器。程序本身绝不下载、不执行、不替换任何东西——一个常驻的透明覆盖层
没有理由去动自己的可执行文件。

- **启动时不联网**：一次请求都不发，只有你点按钮才出去。
- **不添依赖**：用系统自带的 WinHTTP，而不是 `reqwest`（后者带 tokio/rustls，体积会比程序本体还大）。
- **不阻塞界面**：请求在后台线程跑，超时 5/8 秒，失败就当“查不到”，不弹错。

仓库坐标默认是 `Aaze-wu/ScreenCrosshair`。想指向自己 fork 的仓库，两种办法：

- 编译时：`$env:SCREENCROSSHAIR_REPO='你/仓库'; cargo build --release`
- 运行时：在 `config.json` 里写 `"update_repo": "你/仓库"`（优先级更高，改完自动热重载）

两处的值都会被校验（只接受恰好一段 `/`、字符集限于 `A-Za-z0-9._-`），非法值不会拿去发请求。

## 已知限制

- **独占全屏（exclusive fullscreen）的游戏会遮住覆盖层。** 这是 Windows 的显示独占机制，
  覆盖层拿不到画面。解决办法是把游戏改成**无边框窗口（borderless / windowed fullscreen）**，
  这时准星正常显示。
- 只支持 Windows。覆盖层、托盘、热键、开机自启都直接调用 Win32 API，没有跨平台抽象。
- 覆盖层铺在整块屏幕上，某些带反作弊内核驱动的游戏可能把这类窗口视为可疑。自己判断风险。
- 准星画在屏幕**几何中心**，不是游戏内准星的位置。需要挪位置就用热键 `Ctrl+Shift+WASD`、
  托盘菜单的「位置」子菜单，或直接写 `overrides.offset`（单位像素）。

## 开发

```powershell
cargo build            # 调试版（带控制台，能看到热键注册失败之类的日志）
cargo test             # 90 个单测
cargo clippy --all-targets
```

架构是**单进程双窗口**：

- **主线程**跑覆盖层的 Win32 消息循环（`src/platform/overlay.rs`），同时处理热键、托盘、
  配置轮询和重绘。
- **设置窗口**跑在独立线程里（`src/settings.rs`，eframe/egui）。因为 winit 不允许在主线程之外
  建事件循环，用 `EventLoopBuilderExtWindows::with_any_thread` 显式放行。
  又因为 **winit 的事件循环一个进程只能创建一次**，设置窗口永远不销毁，关掉只是隐藏。
- 两个线程通过 `Arc<Mutex<Shared>>` 通信：界面只写 `Shared` 并置 `dirty`，
  真正落盘和重绘都由主循环统一做。**Win32 窗口回调绝不直接碰 `App`。**

渲染用 `tiny-skia` 做 CPU 矢量绘制，画进一块 DIB 后用 `UpdateLayeredWindow` 上传，
所以窗口本身不需要处理 `WM_PAINT`。

### 图标和版本号是怎么进 exe 的

窗口图标（`ViewportBuilder::with_icon`）只影响**窗口**，管不到**文件本身**——exe 的图标
只能来自 PE 的图标资源。所以 `build.rs` 会调 Windows SDK 的 `rc.exe` 把
`assets/app.rc`＋`assets/app.ico` 编成 `.res`，再用 `cargo:rustc-link-arg-bins` 交给链接器。
找不到 `rc.exe` 时只警告不失败（图标缺了顶多丑，不该让编译不过）。

同一个 `build.rs` 还现生成一份 `version.rc`（`VERSIONINFO`），把 `Cargo.toml` 的
`version` 写成 exe 的「文件版本 / 产品版本」。这样 `Cargo.toml` 是版本号唯一的真源：
文件属性、关于界面、安装包三处都从它派生，不会对不上。

> **一次只能给 `rc.exe` 传一个 `.rc`**。图标和版本虽然前后脚编译，但必须分成两次调用、
> 两个 `.res`，再一起交给链接器；直接 `rc.exe a.rc b.rc` 会报
> `fatal error RC1107: invalid usage`（而且只报“用法错”，不告诉你是文件名多了）。

`assets/app.ico` 由 `examples/mkicon.rs` 生成（和运行时用的是同一份矢量绘制代码）：

```powershell
cargo run --example mkicon
```

### `tools/` 自测脚本

实机验证用的小工具（PowerShell 5.1）。**这些脚本必须保存为 UTF-8 带 BOM**，
否则中文注释会被按 GBK 解析导致语法崩。

| 脚本 | 用途 |
| --- | --- |
| `probe.ps1` | 枚举所有窗口（类名 / 可见性 / 矩形 / 扩展样式）+ 显示器信息 |
| `hotkey.ps1` | 注入热键：默认 `Ctrl+Alt+<键>`，`-Shift` 加 Shift，`-NoAlt` 去掉 Alt（发 `Ctrl+Shift+<键>`），`-NoMod` 发裸键 |
| `shot.ps1` | 按坐标截矩形 |
| `winshot.ps1` | 按窗口类名截整窗（`-Class`，可选 `-Title`） |
| `uiaclick.ps1` | 用 UIA 按名字点窗口内控件（不依赖焦点） |
| `trayclick.ps1` | 弹托盘菜单并点某一项（UIA 定位 + 鼠标事件） |
| `traytest.ps1` | 只弹托盘菜单不点（`-NoEnter`），用键盘路径操作 |
| `focus.ps1` | 把窗口置前 |
| `setcfg.ps1` | 改 `config.json`（无 BOM 写入） |
| `snap.ps1` | 按屏幕中心截图（默认 200×200），便捷版 `shot.ps1` |
| `diff.ps1` | 对比两张 PNG，输出变化像素数与包围盒 |
| `clickthrough.ps1` | 验证覆盖层点击穿透（建一个测试窗口压在上面，看点击落到谁身上） |
| `sendinput.ps1` | **合成热键的最小正确实现**（`SendInput` 版），`hotkey.ps1` 的核心 |
| `build-installer.ps1` | 一步出安装包：`cargo build --release` + `ISCC.exe`，产物在 `dist\` |

同一 PowerShell 会话里重复 `Add-Type` 同名类会报「类型名称已存在」而直接退出，
脚本看上去就像「什么都没干」。所以这些脚本一律用独立进程跑：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\hotkey.ps1 -Key up -Shift
```

### ⚠️ 合成热键必须用 `SendInput`，不能用 `keybd_event`

`keybd_event` 是 16 位时代留下的 API。它注入的修饰键**会进入**
`GetAsyncKeyState` 报告的物理键状态（所以自测时看着「注入成功了」），
但**不参与 `RegisterHotKey` 的匹配** —— 系统只看到没有 Shift 的那部分。
后果是带 Shift 的热键在自测里永远落不到自己的 id 上。

实测对照（同一份程序，只换注入方式）：

| 注入方式 | 程序收到的 `WM_HOTKEY` |
| --- | --- |
| `keybd_event` 按住 Shift 再发 ↑ | `id=4`（不透明度）、`shift=false` |
| `SendInput` 按住 Shift 再发 ↑ | `id=6`（缩放）、`shift=true` ✅ |

**产品代码没有任何缺陷** —— 一开始误判成「`Ctrl+Alt+↑` 先注册吃掉了 `Ctrl+Alt+Shift+↑`」，
白改了一版按修饰键数量倒序注册的排序，完全无效（`RegisterHotKey` 自己会区分这两个组合）。
同理，`GetKeyState(VK_SHIFT)` 在 `WM_HOTKEY` 里读不到 Shift：`GetKeyState` 反映的是
**本线程消息队列**的键盘状态，而热键根本不产生键盘消息。**别在 `WM_HOTKEY` 里补救。**

写 `SendInput` 时的两个坑：

- **`INPUT` 结构体在 x64 是 40 字节**（`4 type + 4 padding + 32 union`）。C# 里只声明
  `type` + `ki` 时 `Marshal.SizeOf` 得 32，`SendInput` 会返回 0 且 `GetLastError` = **87**。
  必须 `[StructLayout(LayoutKind.Explicit, Size = 40)]`。
- **方向键是扩展键**，必须置 `KEYEVENTF_EXTENDEDKEY (0x0001)`，否则被当成小键盘 8。
- 顺序也要对：修饰键按下 → 稍等几十毫秒 → 主键按下 → 保持 → 主键抬起 → 修饰键抬起。

### ⚠️ 窗口截图 / 注入点击的坐标是两套

`winshot.ps1` 截的是**窗口矩形**，图里的 `y` 是窗口内相对坐标；注入 `SetCursorPos` 用的是
**屏幕坐标**。所以

```
屏幕坐标 = (窗口 rect.left + 图内 x, 窗口 rect.top + 图内 y)
```

漏加这个原点就会点到隔壁控件上（实测把「位置」点成了「描边」，排查了半天）。另外脚本开头
一律 `SetProcessDPIAware()`，否则 150% 缩放下物理/逻辑像素还会再错一层。

### 常用的两个 Win32 小坑

- **`FindWindowW(cls, "")` 永远匹配不到窗口。** 查不到标题就用 `IntPtr.Zero` /
  `[NullString]::Value`，或者直接 `EnumWindows` 自己过滤——`EnumWindows` 只给可见窗口，
  用 `IsWindowVisible` 再确认一遍。
- **Win11 的托盘图标会被折叠进溢出面板（`^`）。** 想合成一次右键（`WM_TRAY`）不必去点那个像素，
  直接 `PostMessage` 给消息窗口即可；但**弹出来的菜单只认鼠标**，键盘路径需要
  `SetForegroundWindow`，而注入场景下这个调用会被系统拒绝。所以自测一律用鼠标事件。
