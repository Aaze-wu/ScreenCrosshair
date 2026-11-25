# 注入热键。默认发 Ctrl+Alt+<Key>（程序里 10 个热键全是这个前缀），
# 需要裸键时加 -NoMod。
#
# 注意：早期版本把 -CtrlAlt 做成可选开关，默认只发裸字母，
# 结果是「脚本说 sent ctrl+alt o、程序毫无反应」——因为根本没按修饰键。
#
# **必须用 `SendInput`，不能再用 `keybd_event`。**
# `keybd_event` 是 16 位时代的遗留 API，它注入的修饰键虽然会进入
# `GetAsyncKeyState` 的物理键状态（所以看上去“注入成功”），但**不参与
# `RegisterHotKey` 的匹配**。实测证据（程序侧打点）：
#   用 keybd_event 发 Ctrl+Alt+Shift+↑ → 收到 id=4（不透明度），
#                                      且 GetAsyncKeyState(SHIFT)=false
#   用 SendInput   发同一组合      → 收到 id=6（缩放），且 SHIFT=true
# 于是「缩放热键在自测里永远只改不透明度」，会把人骗去改本来正确的产品代码。
param(
    [Parameter(Mandatory = $true)][string]$Key,
    [switch]$Shift,
    [switch]$NoMod,
    # 只按 Ctrl、不按 Alt。位置微调的 1 像素档就是 Ctrl+Shift+WASD，
    # 用它才能跟 10 像素档（Ctrl+Alt+Shift+WASD）区分开。
    [switch]$NoAlt,
    # 主键按住时长（毫秒）。真人按键通常 80~150ms，太短时系统可能来不及
    # 派发 WM_HOTKEY 就收到抬起。
    [int]$HoldMs = 120
)
$ErrorActionPreference = 'Stop'

$sig = @'
using System;
using System.Runtime.InteropServices;

// 类名带前缀：同一个 PowerShell 会话里重复 Add-Type 同名类会直接报
// 「类型名称已存在」而退出，脚本看上去就像“什么都没干”。
public class ScreenCrosshairInput {
    [StructLayout(LayoutKind.Sequential)]
    public struct KEYBDINPUT {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public IntPtr dwExtraInfo;
    }

    // x64 下 INPUT 总长 40 字节：4(type) + 4(padding) + 32(union，最大成员 MOUSEINPUT)。
    // 只算自己用到的字段只能算到 32，必须显式给 Size，否则 SendInput 报错误 87。
    [StructLayout(LayoutKind.Explicit, Size = 40)]
    public struct INPUT {
        [FieldOffset(0)] public uint type;
        [FieldOffset(8)] public KEYBDINPUT ki;
    }

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);

    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();

    public static void Key(ushort vk, bool down, bool extended) {
        INPUT[] inp = new INPUT[1];
        inp[0].type = 1;                                    // INPUT_KEYBOARD
        inp[0].ki.wVk = vk;
        inp[0].ki.wScan = 0;
        inp[0].ki.dwFlags = (down ? 0u : 0x0002u)            // KEYEVENTF_KEYUP
                          | (extended ? 0x0001u : 0u);      // KEYEVENTF_EXTENDEDKEY
        inp[0].ki.time = 0;
        inp[0].ki.dwExtraInfo = IntPtr.Zero;
        if (SendInput(1, inp, Marshal.SizeOf(typeof(INPUT))) != 1) {
            throw new Exception("SendInput 失败，错误码 " + Marshal.GetLastWin32Error());
        }
    }
}
'@
Add-Type -TypeDefinition $sig
[void][ScreenCrosshairInput]::SetProcessDPIAware()

if (-not [Environment]::Is64BitProcess) {
    Write-Output '需要在 64 位 PowerShell 下运行（INPUT 结构大小与位数相关）'
    exit 1
}

function Send-Vk([int]$vk, [bool]$down, [bool]$extended = $false) {
    [ScreenCrosshairInput]::Key([uint16]$vk, $down, $extended)
}

$VK_CONTROL = 0x11
$VK_MENU = 0x12
$VK_SHIFT = 0x10

# 方向键属于「扩展键」：不置 KEYEVENTF_EXTENDEDKEY 会被当成小键盘 8。
$map = @{
    'left' = @(0x25, $true); 'up' = @(0x26, $true)
    'right' = @(0x27, $true); 'down' = @(0x28, $true)
    'h' = @(0x48, $false); 'o' = @(0x4F, $false); 'q' = @(0x51, $false)
    'r' = @(0x52, $false)
    # 位置微调用 WASD（方向键那 4 个 VK 已经被不透明度/尺寸/旋转占满）。
    'w' = @(0x57, $false); 'a' = @(0x41, $false)
    's' = @(0x53, $false); 'd' = @(0x44, $false)
    '0' = @(0x30, $false)
    'esc' = @(0x1B, $false); 'enter' = @(0x0D, $false)
}
$entry = $map[$Key.ToLower()]
if (-not $entry) { Write-Output "unknown key: $Key"; exit 1 }
$vk = [int]$entry[0]
$ext = [bool]$entry[1]

$useMod = -not $NoMod
if ($useMod) {
    Send-Vk $VK_CONTROL $true
    if (-not $NoAlt) { Send-Vk $VK_MENU $true }
}
# Shift 按下后留一点时间，让系统把修饰键状态记进热键匹配。
if ($Shift) { Send-Vk $VK_SHIFT $true; Start-Sleep -Milliseconds 80 }
Send-Vk $vk $true $ext
Start-Sleep -Milliseconds $HoldMs
Send-Vk $vk $false $ext
if ($useMod) {
    if (-not $NoAlt) { Send-Vk $VK_MENU $false }
    Send-Vk $VK_CONTROL $false
}
if ($Shift) { Send-Vk $VK_SHIFT $false }

$prefix = if ($useMod) { if ($NoAlt) { 'ctrl' } else { 'ctrl+alt' } } else { '' }
Write-Output "sent $prefix$(if($Shift){'+shift'}) $Key"
