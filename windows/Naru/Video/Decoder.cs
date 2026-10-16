using System.Runtime.InteropServices;
using Naru.Core;
using SharpGen.Runtime;
using Vortice.Direct3D11;
using Vortice.DXGI;
using Vortice.MediaFoundation;

namespace Naru.Video;

/// <summary>A decoded picture: a slice of a texture the decoder owns until the sample is released, and the band it covers.</summary>
internal readonly record struct Picture(IMFSample Sample, ID3D11Texture2D Texture, uint Slice, Colors Colors, byte Band);

/// <summary>How a stream encodes color: studio (16–235) or full range, and Display P3 or sRGB primaries.</summary>
internal readonly record struct Colors(bool StudioRange, bool DisplayP3);

/// <summary>
/// One Media Foundation decoder for H.264 or HEVC 4:4:4, accelerated through the session's Direct3D device.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Uses Windows' synchronous decoder for the codec, which decodes on the GPU when handed a Direct3D device. Construction fails without that.</item>
/// <item>Each access unit goes in as Annex B, and keyframes get the config's parameter sets in front.</item>
/// <item>H.264 comes out as NV12 and HEVC 4:4:4 as AYUV. The renderer's video processor converts either.</item>
/// <item>With several bands, each picture covers one band of the screen, a picture of its own the size of <see cref="Band"/>.</item>
/// <item>On a new config, dispose and create a new one, since mixing with the previous generation's references corrupts output.</item>
/// <item>Call only on the video thread.</item>
/// </list>
/// </remarks>
internal sealed class Decoder : IDisposable
{
    private static readonly byte[] StartCode = [0, 0, 0, 1];

    private readonly IMFTransform transform;
    private readonly byte[] parameters;
    private readonly Guid output;
    private readonly byte bands;
    private readonly bool displayP3;
    private Colors colors;
    private long decoded;

    public string Name { get; }

    /// <summary>Coded size of one band, or null when one picture covers the screen.</summary>
    public (uint Width, uint Height)? Band { get; }

    public Decoder(Graphics graphics, DecoderConfig config)
    {
        var hevc = config.Codec == VideoCodec.Hevc;
        bands = Math.Max(config.Bands, (byte)1);
        displayP3 = config.DisplayP3;
        var sps = config.Sps.FirstOrDefault() ?? throw new InvalidOperationException("no SPS");
        var sets = hevc ? config.Vps.Concat(config.Sps).Concat(config.Pps) : config.Pps.Prepend(Sps.WithoutReordering(sps));
        parameters = [.. sets.SelectMany(nal => StartCode.Concat(nal))];
        output = hevc ? Mf.Ayuv : Mf.Nv12;

        transform = Create(hevc ? Mf.Hevc : Mf.H264, out var name);
        Name = name;
        var attributes = transform.Attributes;

        if (attributes.GetUInt32(Mf.D3D11Aware) == 0)
        {
            throw new InvalidOperationException($"{Name} cannot decode on the GPU");
        }

        attributes.Set(Mf.LowLatency, 1u);
        transform.ProcessMessage(TMessageType.MessageSetD3DManager, (nuint)graphics.Manager.NativePointer);

        using (var type = InputType(hevc, sps, out var band))
        {
            transform.SetInputType(0, type, 0);
            Band = bands > 1 ? band : null;
        }

        ChooseOutput();

        if ((transform.GetOutputStreamInfo(0).Flags & Mf.ProvidesSamples) == 0)
        {
            throw new InvalidOperationException($"{Name} does not hand out Direct3D surfaces");
        }

        transform.ProcessMessage(TMessageType.MessageNotifyBeginStreaming, 0);
        transform.ProcessMessage(TMessageType.MessageNotifyStartOfStream, 0);
    }

    /// <summary>
    /// Decodes one access unit and returns the pictures it completed. The caller releases each picture's sample.
    /// </summary>
    /// <remarks>The band goes into the sample time, which the decoder copies to the picture it completes.</remarks>
    public List<Picture> Decode(AccessUnit unit)
    {
        var prefix = unit.Keyframe ? parameters.Length : 0;
        var length = prefix + unit.Nals.Sum(nal => StartCode.Length + nal.Length);
        using var buffer = MediaFactory.MFCreateMemoryBuffer(length);
        buffer.Lock(out var pointer, out _, out _);

        try
        {
            var offset = 0;

            void Copy(byte[] bytes)
            {
                Marshal.Copy(bytes, 0, pointer + offset, bytes.Length);
                offset += bytes.Length;
            }

            if (prefix > 0)
            {
                Copy(parameters);
            }

            foreach (var nal in unit.Nals)
            {
                Copy(StartCode);
                Copy(nal);
            }
        }
        finally
        {
            buffer.Unlock();
        }

        buffer.CurrentLength = length;

        using var sample = MediaFactory.MFCreateSample();
        sample.AddBuffer(buffer);
        sample.SampleTime = ++decoded * bands + unit.Band;
        transform.ProcessInput(0, sample, 0);

        return Drain();
    }

    public void Dispose()
    {
        try
        {
            transform.ProcessMessage(TMessageType.MessageCommandFlush, 0);
        }
        catch (SharpGenException)
        {
            // A decoder that already failed may refuse the flush. It is released either way.
        }

        transform.Dispose();
    }

    private List<Picture> Drain()
    {
        var pictures = new List<Picture>();

        while (true)
        {
            var buffer = new OutputDataBuffer { StreamID = 0 };
            var result = transform.ProcessOutput(ProcessOutputFlags.None, 1, ref buffer, out _);
            buffer.Events?.Dispose();

            if (result.Code == Mf.NeedMoreInput)
            {
                return pictures;
            }

            if (result.Code == Mf.StreamChange)
            {
                ChooseOutput();
                continue;
            }

            result.CheckError();

            if (buffer.Sample is not { } sample)
            {
                continue;
            }

            using var media = sample.GetBufferByIndex(0);
            using var surface = media.QueryInterface<IMFDXGIBuffer>();
            var texture = new ID3D11Texture2D(surface.GetResource(typeof(ID3D11Texture2D).GUID));
            pictures.Add(new Picture(sample, texture, surface.SubresourceIndex, colors, (byte)(sample.SampleTime % bands)));
        }
    }

    // Picks the codec's output format. The type also says whether the stream uses studio range; the config says the primaries.
    private void ChooseOutput()
    {
        for (var index = 0; ; index++)
        {
            using var type = transform.GetOutputAvailableType(0, index);

            if (type.GetGUID(Mf.Subtype) == output)
            {
                var studioRange = MediaFactory.MFGetAttributeUInt32(type, Mf.NominalRange, Mf.NominalRange16To235) == Mf.NominalRange16To235;
                colors = new Colors(studioRange, displayP3);
                transform.SetOutputType(0, type, 0);
                return;
            }
        }
    }

    private static IMFTransform Create(Guid codec, out string name)
    {
        var input = new RegisterTypeInfo { GuidMajorType = Mf.Video, GuidSubtype = codec };
        using var found = MediaFactory.MFTEnumEx(Mf.VideoDecoderCategory, Mf.EnumSyncMft | Mf.EnumLocalMft | Mf.EnumSortAndFilter, input, null);
        var activate = found.FirstOrDefault() ?? throw new InvalidOperationException($"no decoder for {codec}");
        name = activate.GetString(TransformAttributeKeys.MftFriendlyNameAttribute);
        return activate.ActivateObject<IMFTransform>();
    }

    // HEVC declares the 4:4:4 profile and the frame size up front, or the decoder offers no 4:4:4 output.
    private static IMFMediaType InputType(bool hevc, byte[] sps, out (uint Width, uint Height)? band)
    {
        band = null;
        var type = MediaFactory.MFCreateMediaType();
        type.Set(Mf.MajorType, Mf.Video);
        type.Set(Mf.Subtype, hevc ? Mf.Hevc : Mf.H264);
        type.Set(Mf.InterlaceMode, Mf.InterlaceProgressive);

        if (hevc)
        {
            var parsed = Sps.Hevc(sps) ?? throw new InvalidOperationException("cannot read the HEVC SPS");

            if (parsed.ChromaFormat != 3)
            {
                throw new InvalidOperationException($"HEVC chroma format {parsed.ChromaFormat} is not 4:4:4");
            }

            MediaFactory.MFSetAttributeSize(type, Mf.FrameSize, parsed.Width, parsed.Height);
            band = (parsed.Width, parsed.Height);
            type.Set(Mf.VideoProfile, Mf.HevcMain444);
        }

        return type;
    }

    // HEVC 4:4:4 8-bit parameter sets a Mac sent for a 2420×1668 virtual display, the same ones the iPad app checks with.
    private static readonly byte[] MacVps =
    [
        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00,
        0x00, 0x96, 0x0c, 0xc0, 0x90,
    ];

    private static readonly byte[] MacSps =
    [
        0x42, 0x01, 0x01, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00, 0x00, 0x96, 0x90,
        0x00, 0x26, 0x04, 0x00, 0xd2, 0x38, 0xd8, 0xdc, 0x40, 0x67, 0x72, 0x29, 0x0c, 0x5c, 0xfe, 0x27, 0xe1, 0x7f,
        0x43, 0x7e, 0xa1, 0xff, 0x54, 0x11, 0xfa, 0xa8, 0x27, 0xf5, 0x54, 0x15, 0xfa, 0xaa, 0x82, 0xff, 0x55, 0x54,
        0x19, 0xfa, 0xaa, 0xa9, 0x4d, 0xc3, 0x03, 0x40, 0x40, 0x20,
    ];

    private static readonly byte[] MacPps = [0x44, 0x01, 0xc0, 0x72, 0xf0, 0x5b, 0x24];

    /// <summary>Whether this PC can decode the Mac's HEVC 4:4:4 in hardware. Without it, the session uses H.264.</summary>
    /// <remarks>
    /// <para>
    /// Windows documents its HEVC decoder as 4:2:0 only, but with HEVC Video Extensions installed it decodes 4:4:4 to
    /// AYUV on GPUs that support it. That is undocumented, so every link is checked:
    /// </para>
    /// <list type="bullet">
    /// <item>The GPU has the HEVC Main 4:4:4 decode profile with AYUV output. The extension's software fallback would also accept 4:4:4, so this keeps decoding on hardware.</item>
    /// <item>The video processor takes AYUV input.</item>
    /// <item>The decoder, set up with parameter sets a Mac sent, offers AYUV output.</item>
    /// </list>
    /// <para>If HEVC video still does not start, the core retries the session with H.264.</para>
    /// </remarks>
    public static bool SupportsHevc444() => Hevc444.Value;

    private static readonly Lazy<bool> Hevc444 = new(() =>
    {
        try
        {
            using var graphics = new Graphics();
            using var video = graphics.Device.QueryInterface<ID3D11VideoDevice>();

            if (!video.CheckVideoDecoderFormat(Mf.DecoderProfileHevcMain444, Format.AYUV))
            {
                Log.Info("HEVC 4:4:4: the GPU has no AYUV decode profile");
                return false;
            }

            using (var enumerator = video.CreateVideoProcessorEnumerator(new VideoProcessorContentDescription
            {
                InputFrameFormat = VideoFrameFormat.Progressive,
                InputWidth = 1920,
                InputHeight = 1080,
                OutputWidth = 1920,
                OutputHeight = 1080,
                Usage = VideoUsage.OptimalSpeed,
            }))
            {
                if ((enumerator.CheckVideoProcessorFormat(Format.AYUV) & VideoProcessorFormatSupport.Input) == 0)
                {
                    Log.Info("HEVC 4:4:4: the video processor does not take AYUV");
                    return false;
                }
            }

            var config = new DecoderConfig(VideoCodec.Hevc, 1, 2420, 1668, true, [MacVps], [MacSps], [MacPps]);
            using var decoder = new Decoder(graphics, config);
            Log.Info($"HEVC 4:4:4: supported by {decoder.Name}");
            return true;
        }
        catch (Exception error) when (error is SharpGenException or InvalidOperationException or COMException)
        {
            Log.Info($"HEVC 4:4:4: unsupported ({error.Message})");
            return false;
        }
    });
}
