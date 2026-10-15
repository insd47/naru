using System.Runtime.InteropServices;

namespace Naru.Session;

/// <summary>Win32 functions and constants for keyboard capture, keyboard layouts, the IME, and wheel settings.</summary>
internal static unsafe partial class Native
{
    public const int WhKeyboardLl = 13;
    public const int WmKeyDown = 0x0100;
    public const int WmSysKeyDown = 0x0104;
    public const uint LlkhfExtended = 0x01;
    public const uint LlkhfInjected = 0x10;
    public const uint ImeCmodeNative = 0x1;

    public const uint VkCancel = 0x03;
    public const uint VkBack = 0x08;
    public const uint VkTab = 0x09;
    public const uint VkReturn = 0x0d;
    public const uint VkShift = 0x10;
    public const uint VkPause = 0x13;
    public const uint VkCapital = 0x14;
    public const uint VkHangul = 0x15;
    public const uint VkHanja = 0x19;
    public const uint VkEscape = 0x1b;
    public const uint VkSpace = 0x20;
    public const uint VkPrior = 0x21;
    public const uint VkNext = 0x22;
    public const uint VkEnd = 0x23;
    public const uint VkHome = 0x24;
    public const uint VkLeft = 0x25;
    public const uint VkUp = 0x26;
    public const uint VkRight = 0x27;
    public const uint VkDown = 0x28;
    public const uint VkInsert = 0x2d;
    public const uint VkDelete = 0x2e;
    public const uint VkLWin = 0x5b;
    public const uint VkRWin = 0x5c;
    public const uint VkLShift = 0xa0;
    public const uint VkRShift = 0xa1;
    public const uint VkLControl = 0xa2;
    public const uint VkRControl = 0xa3;
    public const uint VkLMenu = 0xa4;
    public const uint VkRMenu = 0xa5;

    public const uint SpiGetWheelScrollLines = 0x0068;
    public const uint SpiGetWheelScrollChars = 0x006C;
    private const uint WheelPageScroll = uint.MaxValue;

    [StructLayout(LayoutKind.Sequential)]
    public struct KbdLlHookStruct
    {
        public uint VkCode;
        public uint ScanCode;
        public uint Flags;
        public uint Time;
        public nuint ExtraInfo;
    }

    [LibraryImport("user32.dll", EntryPoint = "SetWindowsHookExW", SetLastError = true)]
    public static partial nint SetWindowsHookEx(int hook, delegate* unmanaged<int, nint, nint, nint> procedure, nint module, uint thread);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool UnhookWindowsHookEx(nint hook);

    [LibraryImport("user32.dll")]
    public static partial nint CallNextHookEx(nint hook, int code, nint wParam, nint lParam);

    [LibraryImport("kernel32.dll", EntryPoint = "GetModuleHandleW", StringMarshalling = StringMarshalling.Utf16)]
    public static partial nint GetModuleHandle(string? name);

    [LibraryImport("user32.dll")]
    public static partial nint GetForegroundWindow();

    [LibraryImport("user32.dll")]
    public static partial uint GetWindowThreadProcessId(nint window, nint processId);

    [LibraryImport("user32.dll")]
    public static partial nint GetKeyboardLayout(uint thread);

    [LibraryImport("user32.dll")]
    public static partial int ToUnicodeEx(uint key, uint scan, byte* state, char* buffer, int size, uint flags, nint layout);

    [LibraryImport("imm32.dll")]
    public static partial nint ImmGetContext(nint window);

    [LibraryImport("imm32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool ImmGetConversionStatus(nint context, out uint conversion, out uint sentence);

    [LibraryImport("imm32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool ImmReleaseContext(nint window, nint context);

    /// <summary>Lines or characters a wheel notch scrolls, 3 by default</summary>
    /// <remarks>"One screen at a time" has no counterpart on the Mac, so it scrolls the default.</remarks>
    public static uint WheelLines(uint setting)
    {
        uint lines = 3;
        return SystemParametersInfo(setting, 0, &lines, 0) && lines != WheelPageScroll ? lines : 3;
    }

    [LibraryImport("user32.dll", EntryPoint = "SystemParametersInfoW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool SystemParametersInfo(uint action, uint param, uint* value, uint update);
}
