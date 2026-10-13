using System.Runtime.InteropServices;

namespace Naru.Session;

/// <summary>Win32 functions for keyboard capture, the IME, wheel settings, and the cursor.</summary>
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

    public const uint SpiGetWheelScrollLines = 0x0068;
    public const uint SpiGetWheelScrollChars = 0x006C;
    public const uint WheelPageScroll = uint.MaxValue;

    [LibraryImport("user32.dll", EntryPoint = "SystemParametersInfoW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool SystemParametersInfo(uint action, uint param, uint* value, uint update);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool DestroyIcon(nint icon);

    [StructLayout(LayoutKind.Sequential)]
    private struct BitmapInfoHeader
    {
        public uint Size;
        public int Width;
        public int Height;
        public ushort Planes;
        public ushort BitCount;
        public uint Compression;
        public uint SizeImage;
        public int XPelsPerMeter;
        public int YPelsPerMeter;
        public uint ClrUsed;
        public uint ClrImportant;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct IconInfo
    {
        public int Icon;
        public uint HotspotX;
        public uint HotspotY;
        public nint Mask;
        public nint Color;
    }

    [LibraryImport("gdi32.dll")]
    private static partial nint CreateDIBSection(nint dc, BitmapInfoHeader* info, uint usage, void** bits, nint section, uint offset);

    [LibraryImport("gdi32.dll")]
    private static partial nint CreateBitmap(int width, int height, uint planes, uint bitCount, void* bits);

    [LibraryImport("gdi32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool DeleteObject(nint bitmap);

    [LibraryImport("user32.dll")]
    private static partial nint CreateIconIndirect(IconInfo* info);

    /// <summary>A cursor from top-down BGRA pixels with straight alpha. The alpha draws it, so the mask is empty.</summary>
    public static nint CreateCursor(byte[] bgra, int width, int height, uint hotspotX, uint hotspotY)
    {
        var header = new BitmapInfoHeader
        {
            Size = (uint)sizeof(BitmapInfoHeader),
            Width = width,
            Height = -height,
            Planes = 1,
            BitCount = 32,
        };
        void* bits;
        var color = CreateDIBSection(0, &header, 0, &bits, 0, 0);

        if (color == 0)
        {
            return 0;
        }

        Marshal.Copy(bgra, 0, (nint)bits, bgra.Length);

        // Monochrome rows are padded to 16 bits. All zero, the mask leaves every pixel to the alpha.
        var zeros = new byte[(width + 15) / 16 * 2 * height];
        nint mask;

        fixed (byte* pointer = zeros)
        {
            mask = CreateBitmap(width, height, 1, 1, pointer);
        }

        var info = new IconInfo { Icon = 0, HotspotX = hotspotX, HotspotY = hotspotY, Mask = mask, Color = color };
        var cursor = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color);
        return cursor;
    }

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
