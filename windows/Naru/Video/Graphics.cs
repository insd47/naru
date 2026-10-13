using Vortice.Direct3D;
using Vortice.Direct3D11;
using Vortice.MediaFoundation;

namespace Naru.Video;

/// <summary>
/// The Direct3D 11 device one session decodes and presents with, shared with Media Foundation.
/// </summary>
/// <remarks>Media Foundation calls the device from its own threads, so multithread protection is on.</remarks>
internal sealed class Graphics : IDisposable
{
    private static readonly Lazy<bool> Started = new(() => MediaFactory.MFStartup(true).Success);

    public ID3D11Device Device { get; }
    public ID3D11DeviceContext Context { get; }
    public IMFDXGIDeviceManager Manager { get; }

    public Graphics()
    {
        if (!Started.Value)
        {
            throw new InvalidOperationException("Media Foundation did not start");
        }

        D3D11.D3D11CreateDevice(
            IntPtr.Zero,
            DriverType.Hardware,
            DeviceCreationFlags.VideoSupport | DeviceCreationFlags.BgraSupport,
            [FeatureLevel.Level_11_1, FeatureLevel.Level_11_0],
            out var device,
            out var context).CheckError();
        Device = device!;
        Context = context!;

        using (var multithread = Device.QueryInterface<ID3D11Multithread>())
        {
            multithread.SetMultithreadProtected(true);
        }

        Manager = MediaFactory.MFCreateDXGIDeviceManager();
        Manager.ResetDevice(Device).CheckError();
    }

    public void Dispose()
    {
        Manager.Dispose();
        Context.Dispose();
        Device.Dispose();
    }
}

/// <summary>Media Foundation identifiers Vortice does not name.</summary>
internal static class Mf
{
    public static readonly Guid VideoDecoderCategory = new("d6c02d4b-6833-45b4-971a-05a4b04bab91");
    public static readonly Guid MajorType = new("48eba18e-f8c9-4687-bf11-0a74c9f96a8f");
    public static readonly Guid Subtype = new("f7e34c9a-42e8-4714-b74b-cb29d72c35e5");
    public static readonly Guid FrameSize = new("1652c33d-d6b2-4012-b834-72030849a37d");
    public static readonly Guid InterlaceMode = new("e2724bb8-e676-4806-b4b2-a8d6efb44ccd");
    public static readonly Guid NominalRange = new("c21b8ee5-b956-4071-8daf-325edf5cab11");
    public static readonly Guid Video = new("73646976-0000-0010-8000-00aa00389b71");
    public static readonly Guid H264 = new("34363248-0000-0010-8000-00aa00389b71");
    public static readonly Guid Nv12 = new("3231564e-0000-0010-8000-00aa00389b71");
    public static readonly Guid Hevc = new("43564548-0000-0010-8000-00aa00389b71");
    public static readonly Guid Ayuv = new("56555941-0000-0010-8000-00aa00389b71");
    public static readonly Guid VideoProfile = new("ad76a80b-2d5c-4e0b-b375-64e520137036");
    public static readonly Guid DecoderProfileHevcMain444 = new("4008018f-f537-4b36-98cf-61af8a2c1a33");
    public static readonly Guid D3D11Aware = new("206b4fc8-fcf9-4c51-afe3-9764369e33a0");
    public static readonly Guid LowLatency = new("9c27891a-ed7a-40e1-88e8-b22727a024ee");

    public const uint EnumSyncMft = 0x1;
    public const uint EnumLocalMft = 0x10;
    public const uint EnumSortAndFilter = 0x40;
    public const uint InterlaceProgressive = 2;
    public const uint NominalRange16To235 = 2;
    public const uint HevcMain444 = 6;
    public const int ProvidesSamples = 0x100;

    public const int NeedMoreInput = unchecked((int)0xC00D6D72);
    public const int StreamChange = unchecked((int)0xC00D6D61);
}
