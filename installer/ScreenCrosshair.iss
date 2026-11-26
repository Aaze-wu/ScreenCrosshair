; ============================================================================
;  ScreenCrosshair 安装包脚本
;
;  编译：
;    ISCC.exe installer\ScreenCrosshair.iss
;  或（会先 cargo build --release）：
;    powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
;
;  ---------------------------------------------------------------------------
;  版本号不写在这个文件里
;  ---------------------------------------------------------------------------
;  `AppVersion` 是从 `target\release\screencrosshair.exe` 的 VERSIONINFO 读出来的，
;  而那个值由 `build.rs` 从 `Cargo.toml` 的 `version` 生成。
;  所以版本号只有一个真源（Cargo.toml），安装包、文件属性、关于界面永远一致。
;  代价是**必须先 cargo build --release**，否则这里直接报错退出。
;
;  ---------------------------------------------------------------------------
;  为什么要绕开 AppMutex
;  ---------------------------------------------------------------------------
;  常规做法是设 `AppMutex=`，让安装程序在启动时就发现「程序正在运行」并弹窗
;  要求用户关闭。这个程序上那招等于死路：覆盖层是 WS_EX_TOOLWINDOW 的透明层，
;  **任务栏里没有任何窗口**，用户看到提示后根本找不到可以关的东西
;  （只能摸到托盘图标，而多数人不知道它在那儿）。
;
;  这里改成主动请它退出：`PrepareToInstall` 阶段调用新 exe 的 `--quit`，
;  由它给正在运行的旧实例投递一条 WM_APP 消息，旧实例自己走人。
; ============================================================================

#define AppName "ScreenCrosshair"
#define AppExeName "screencrosshair.exe"

; 源码根目录（本文件在 installer\ 下）
#define SourceRoot AddBackslash(SourcePath) + ".."
#define AppExe SourceRoot + "\target\release\" + AppExeName

; ISPP 的逻辑非是 `!`，不是 `not`（写 `not` 会报 Undeclared identifier）
#if !FileExists(AppExe)
  #error 找不到 target\release\screencrosshair.exe，请先执行 cargo build --release
#endif

; VERSIONINFO 里的字符串版本（如 0.1.0）与四段数字版本（0.1.0.0）
#define AppVersion GetStringFileInfo(AppExe, FILE_VERSION)
#define AppVersionFull GetVersionNumbersString(AppExe)

; 仓库地址：和 update.rs / Cargo.toml 保持一致，但允许用环境变量覆盖
; （想把安装包指向自己 fork 的仓库时就设 SCREENCROSSHAIR_REPO）。
#define RepoSlug GetEnv("SCREENCROSSHAIR_REPO")
#if RepoSlug == ""
  #define RepoSlug "Aaze-wu/ScreenCrosshair"
#endif
#define AppUrl "https://github.com/" + RepoSlug

[Setup]
; AppId 决定「这是同一个软件的升级」还是「另装一份」。**一旦发布就不能改**，
; 改了会让老版本无法被覆盖安装、卸载列表里出现两个条目。
AppId={{7C3E1B4A-9D2F-4A6B-8C55-1E9F3A7B2D64}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
VersionInfoVersion={#AppVersionFull}
VersionInfoProductVersion={#AppVersionFull}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} 屏幕中心准星 安装程序
VersionInfoCopyright=AGPL-3.0

; ---- 安装位置与权限 ----
; 默认按用户安装（不需要管理员，装到 %LOCALAPPDATA%\Programs）：这个程序
; 只有一个 exe + 一个 JSON 配置，没有任何需要管理员的东西。
; `PrivilegesRequiredOverridesAllowed=dialog` 让安装程序在开头多一页，
; 想装给「所有用户」的人可以在那里切换（那份走 {autopf} → Program Files）。
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes

; ---- 输出 ----
OutputDir={#SourceRoot}\dist
OutputBaseFilename={#AppName}-{#AppVersion}-setup
SetupIconFile={#SourceRoot}\assets\app.ico
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName} {#AppVersion}
OutputManifestFile={#AppName}-setup-manifest.txt

; ---- 杂项 ----
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
; 程序用到分层窗口 / DPI API，Windows 10 以下没有验证过，直接不装。
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile={#SourceRoot}\LICENSE
; 同一时间只允许一个安装程序（两个并排跑会互相覆盖文件）
SetupMutex=ScreenCrosshair.Setup
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "zhcn"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
zhcn.AutoStartTask=开机自动启动（登录 Windows 后自动显示准星）
en.AutoStartTask=Start {#AppName} when I sign in
zhcn.LaunchProgram=立即运行 {#AppName}
en.LaunchProgram=Launch {#AppName}
zhcn.DeleteConfigTask=卸载时同时删除配置文件（%APPDATA%\ScreenCrosshair）
en.DeleteConfigTask=Also delete my settings (%APPDATA%\ScreenCrosshair)

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
; 不加 `unchecked`：自启是这个程序最常见的用法（准星本来就该一直在）。
Name: "autostart"; Description: "{cm:AutoStartTask}"

[Files]
; 主程序。`ignoreversion` 是必须的：我们每次构建都可能版本号不变而内容变了，
; 让安装程序按版本号判断会导致「同版本覆盖安装时文件根本没被换掉」。
Source: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
; 仅供 PrepareToInstall 阶段「请走正在运行的旧实例」使用。
; dontcopy = 只解到 {tmp}，不落到安装目录。必须单独列一条：同一条目里
; 不能既指定 {app} 又指定 {tmp}。
Source: "{#AppExe}"; DestDir: "{tmp}"; Flags: dontcopy

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon
; 开机自启落在「启动」文件夹——**不要**写注册表 Run 项：某些机器上
; 电脑管家一类的安全软件会把 Run 值当可疑项清掉（本项目实测踩过），
; 而按用户/按路径写入的 .lnk 不会被清理。这也是程序自身在设置界面里
; 开自启时用的同一个位置，两者不会打架。
Name: "{userstartup}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: autostart

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 程序自己开的自启（设置界面里的开关）指向的是 {app}，卸载后就成了死链接。
; 安装程序创建的那份会被自动清掉，这一条是给「用户在程序里自己开的」兜底。
Type: files; Name: "{userstartup}\{#AppName}.lnk"

[Code]
// 请走正在运行的实例。
//
// 覆盖层没有任务栏窗口，用户无法通过常规方式关闭它，而安装程序要替换的
// 正是它正在执行的 exe（Windows 允许覆盖正在运行的 exe，但正在重新启动的
// 那份会读到半个文件，且旧进程会一直占着托盘图标和全局热键）。
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
  Helper: String;
begin
  Result := '';
  ExtractTemporaryFile('{#AppExeName}');
  Helper := ExpandConstant('{tmp}\{#AppExeName}');
  if FileExists(Helper) then
  begin
    Exec(Helper, '--quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    // 它退出时还要删托盘图标、注销热键，给它一点时间；否则紧接着的
    // 文件替换可能撞上还没释放的句柄。
    Sleep(500);
  end;
end;

// 卸载时同样要先结束程序，否则 {app} 里的 exe 被占用，卸载会留下一堆残骸。
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
  Exe: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    Exe := ExpandConstant('{app}\{#AppExeName}');
    if FileExists(Exe) then
    begin
      Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Sleep(500);
    end;
  end;
end;
