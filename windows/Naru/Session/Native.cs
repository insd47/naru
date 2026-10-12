using System.Runtime.InteropServices;

namespace Naru.Session;

/// <summary>Win32 functions for keyboard capture and the IME.</summary>
internal static unsafe partial class Native
{
    public const int WhKeyboardLl = 13;
    public const int WmKeyDown = 0x0100;
    public const int WmSysKeyDown = 0x0104;
    public const uint LlkhfExtended = 0x01;
    public const uint LlkhfInjected = 0x10;
    public const uint ImeCmodeNative = 0x1;

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

    [LibraryImport("user32.dll")]
    public static partial nint GetForegroundWindow();

    [LibraryImport("user32.dll")]
    public static partial uint GetWindowThreadProcessId(nint window, nint processId);

    [LibraryImport("user32.dll")]
    public static partial nint GetKeyboardLayout(uint thread);

    [LibraryImport("user32.dll")]
    public static partial int ToUnicodeEx(uint key, uint scan, byte* state, char* buffer, int size, uint flags, nint layout);

    [LibraryImport("kernel32.dll", EntryPoint = "GetModuleHandleW", StringMarshalling = StringMarshalling.Utf16)]
    public static partial nint GetModuleHandle(string? name);

    [LibraryImport("imm32.dll")]
    public static partial nint ImmGetContext(nint window);

    [LibraryImport("imm32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool ImmGetConversionStatus(nint context, out uint conversion, out uint sentence);

    [LibraryImport("imm32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool ImmReleaseContext(nint window, nint context);
}
