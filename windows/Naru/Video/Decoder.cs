using System.Runtime.InteropServices;
using Naru.Core;
using SharpGen.Runtime;
using Vortice.Direct3D11;
using Vortice.MediaFoundation;

namespace Naru.Video;

/// <summary>A decoded picture: a slice of a texture the decoder owns until the sample is released.</summary>
internal readonly record struct Picture(IMFSample Sample, ID3D11Texture2D Texture, uint Slice, bool StudioRange);

/// <summary>
/// One Media Foundation H.264 decoder, accelerated through the session's Direct3D device.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Uses Windows' built-in synchronous decoder, which decodes on the GPU when handed a Direct3D device. Construction fails without that.</item>
/// <item>Each access unit goes in as Annex B. Keyframes carry the SPS and PPS in front, so the decoder needs no separate setup.</item>
/// <item>On a new SPS or PPS, dispose and create a new one, since mixing with the previous generation's references corrupts output.</item>
/// <item>Call only on the video thread.</item>
/// </list>
/// </remarks>
internal sealed class Decoder : IDisposable
{
    private static readonly byte[] StartCode = [0, 0, 0, 1];

    private readonly IMFTransform transform;
    private readonly byte[] parameters;
    private bool studioRange = true;

    public string Name { get; }

    public Decoder(Graphics graphics, byte[] sps, byte[] pps)
    {
        parameters = [.. StartCode, .. Sps.WithoutReordering(sps), .. StartCode, .. pps];

        var input = new RegisterTypeInfo { GuidMajorType = Mf.Video, GuidSubtype = Mf.H264 };
        using var found = MediaFactory.MFTEnumEx(Mf.VideoDecoderCategory, Mf.EnumSyncMft | Mf.EnumLocalMft | Mf.EnumSortAndFilter, input, null);
        var activate = found.FirstOrDefault() ?? throw new InvalidOperationException("no H.264 decoder");
        Name = activate.GetString(TransformAttributeKeys.MftFriendlyNameAttribute);
        transform = activate.ActivateObject<IMFTransform>();

        var attributes = transform.Attributes;

        if (attributes.GetUInt32(Mf.D3D11Aware) == 0)
        {
            throw new InvalidOperationException($"{Name} cannot decode on the GPU");
        }

        attributes.Set(Mf.LowLatency, 1u);
        transform.ProcessMessage(TMessageType.MessageSetD3DManager, (nuint)graphics.Manager.NativePointer);

        using (var type = MediaFactory.MFCreateMediaType())
        {
            type.Set(Mf.MajorType, Mf.Video);
            type.Set(Mf.Subtype, Mf.H264);
            type.Set(Mf.InterlaceMode, Mf.InterlaceProgressive);
            transform.SetInputType(0, type, 0);
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
    /// <remarks>RTP timestamps are 90 kHz. They only order output, so wraparound does not matter.</remarks>
    public List<Picture> Decode(AccessUnit unit)
    {
        var length = (unit.Keyframe ? parameters.Length : 0) + unit.Nals.Sum(nal => StartCode.Length + nal.Length);
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

            if (unit.Keyframe)
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
        sample.SampleTime = unit.Timestamp * 1000L / 9;
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
            var output = new OutputDataBuffer { StreamID = 0 };
            var result = transform.ProcessOutput(ProcessOutputFlags.None, 1, ref output, out _);
            output.Events?.Dispose();

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

            if (output.Sample is not { } sample)
            {
                continue;
            }

            using var buffer = sample.GetBufferByIndex(0);
            using var surface = buffer.QueryInterface<IMFDXGIBuffer>();
            var texture = new ID3D11Texture2D(surface.GetResource(typeof(ID3D11Texture2D).GUID));
            pictures.Add(new Picture(sample, texture, surface.SubresourceIndex, studioRange));
        }
    }

    // Picks NV12, which the video processor converts. The type also says whether the stream uses studio range.
    private void ChooseOutput()
    {
        for (var index = 0; ; index++)
        {
            using var type = transform.GetOutputAvailableType(0, index);

            if (type.GetGUID(Mf.Subtype) == Mf.Nv12)
            {
                studioRange = MediaFactory.MFGetAttributeUInt32(type, Mf.NominalRange, Mf.NominalRange16To235) == Mf.NominalRange16To235;
                transform.SetOutputType(0, type, 0);
                return;
            }
        }
    }

    /// <summary>Whether this PC can decode the Mac's HEVC 4:4:4 in hardware. Without it, the session uses H.264.</summary>
    /// <remarks>Not checked yet: HEVC 4:4:4 needs a GPU decode profile query and a different decoder, so H.264 is used everywhere.</remarks>
    public static bool SupportsHevc444() => false;
}
