# 用官方 `SendInput` 注入 Ctrl+Alt+Shift+↑，与 keybd_event 版本对照。
#
# 目的：`keybd_event` 是遗留 API，它注入的修饰键**可能**不参与 `RegisterHotKey`
# 的匹配（系统只把物理键状态记下来，热键匹配另走一套）。若本脚本能让程序
# 正确收到缩放热键，说明问题是 keybd_event 的局限，而不是产品逻辑。
$ErrorActionPreference = 'Stop'

Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public class SI {
    [StructLayout(LayoutKind.Sequential)]
    public struct KEYBDINPUT {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public IntPtr dwExtraInfo;
    }

    // x64 下 INPUT 总长 40 字节：4(type) + 4(padding) + 32(union，最大成员 MOUSEINPUT)。
    // 自己算字段只能算到 32，必须显式给 Size，否则 SendInput 返回错误 87。
    [StructLayout(LayoutKind.Explicit, Size = 40)]
    public struct INPUT {
        [FieldOffset(0)] public uint type;
        [FieldOffset(8)] public KEYBDINPUT ki;
    }

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);

    [DllImport("user32.dll")]
    public static extern short GetAsyncKeyState(int vKey);

    const uint KEYEVENTF_EXTENDEDKEY = 0x0001;
    const uint KEYEVENTF_KEYUP = 0x0002;

    public static void Key(ushort vk, bool down, bool extended) {
        INPUT[] inp = new INPUT[1];
        inp[0].type = 1;                                   // INPUT_KEYBOARD
        inp[0].ki.wVk = vk;
        inp[0].ki.wScan = 0;
        inp[0].ki.dwFlags = (down ? 0u : KEYEVENTF_KEYUP)
                          | (extended ? KEYEVENTF_EXTENDEDKEY : 0u);
        inp[0].ki.time = 0;
        inp[0].ki.dwExtraInfo = IntPtr.Zero;
        uint sent = SendInput(1, inp, Marshal.SizeOf(typeof(INPUT)));
        if (sent != 1) {
            throw new Exception("SendInput 失败，错误码 " + Marshal.GetLastWin32Error());
        }
    }
}
'@

$VK_SHIFT = 0x10
$VK_CONTROL = 0x11
$VK_MENU = 0x12
$VK_UP = 0x26

Write-Output ("is64={0} inputSize={1}" -f [Environment]::Is64BitProcess, [System.Runtime.InteropServices.Marshal]::SizeOf([type][SI+INPUT]))

# 修饰键不是扩展键；方向键是扩展键（不加会变成小键盘 8）。
[SI]::Key($VK_CONTROL, $true, $false)
[SI]::Key($VK_MENU, $true, $false)
[SI]::Key($VK_SHIFT, $true, $false)
Start-Sleep -Milliseconds 200
Write-Output ("armed: shift={0}" -f ([SI]::GetAsyncKeyState($VK_SHIFT) -lt 0))

[SI]::Key($VK_UP, $true, $true)
Start-Sleep -Milliseconds 400
[SI]::Key($VK_UP, $false, $true)

[SI]::Key($VK_MENU, $false, $false)
[SI]::Key($VK_CONTROL, $false, $false)
Start-Sleep -Milliseconds 100
[SI]::Key($VK_SHIFT, $false, $false)

Write-Output 'sent ctrl+alt+shift up (SendInput)'
