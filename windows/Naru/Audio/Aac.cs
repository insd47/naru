using System.Runtime.InteropServices;

namespace Naru.Audio;

/// <summary>
/// An AAC-ELD decoder from the Fraunhofer FDK AAC library.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Windows' AAC decoder rejects AAC-ELD, and FFmpeg's has no low delay SBR, which the Mac uses, so FDK is the decoder that plays it.</item>
/// <item>Each access unit comes without a transport header, so the decoder is configured with the AudioSpecificConfig.</item>
/// <item>Not thread-safe. The PCM it returns is valid until the next call.</item>
/// </list>
/// </remarks>
internal sealed partial class Aac : IDisposable
{
    private const string Library = "libAACdec";
    private const int RawTransport = 0;

    // Large enough for any frame the decoder can produce: 2048 samples per channel, 8 channels.
    private readonly short[] pcm = new short[2048 * 8];
    private nint handle;

    public Aac(byte[] config)
    {
        handle = aacDecoder_Open(RawTransport, 1);

        if (handle == 0)
        {
            throw new InvalidOperationException("Cannot open the AAC decoder");
        }

        unsafe
        {
            fixed (byte* bytes = config)
            {
                var pointer = bytes;
                var length = (uint)config.Length;
                Check(aacDecoder_ConfigRaw(handle, &pointer, &length), "configure");
            }
        }
    }

    public int SampleRate { get; private set; }
    public int Channels { get; private set; }

    /// <summary>Decodes one access unit into interleaved 16-bit PCM, or returns nothing for a frame it cannot decode.</summary>
    public unsafe ReadOnlySpan<short> Decode(byte[] unit)
    {
        int error;

        fixed (byte* bytes = unit)
        {
            var pointer = bytes;
            var size = (uint)unit.Length;
            var valid = size;
            error = aacDecoder_Fill(handle, &pointer, &size, &valid);
        }

        if (error == 0)
        {
            fixed (short* samples = pcm)
            {
                error = aacDecoder_DecodeFrame(handle, samples, pcm.Length, 0);
            }
        }

        // Lost or damaged frames come back as errors. They are skipped, and the next frame decodes on its own.
        if (error != 0)
        {
            return [];
        }

        var info = aacDecoder_GetStreamInfo(handle);
        SampleRate = info->sampleRate;
        Channels = info->numChannels;
        return pcm.AsSpan(0, info->frameSize * info->numChannels);
    }

    public void Dispose()
    {
        if (handle != 0)
        {
            aacDecoder_Close(handle);
            handle = 0;
        }
    }

    private static void Check(int error, string what)
    {
        if (error != 0)
        {
            throw new InvalidOperationException($"Cannot {what} the AAC decoder: 0x{error:x}");
        }
    }

    // The leading fields of FDK's CStreamInfo
    [StructLayout(LayoutKind.Sequential)]
    private struct StreamInfo
    {
        public int sampleRate;
        public int frameSize;
        public int numChannels;
    }

    [LibraryImport(Library)]
    private static partial nint aacDecoder_Open(int transport, uint layers);

    [LibraryImport(Library)]
    private static unsafe partial int aacDecoder_ConfigRaw(nint decoder, byte** config, uint* length);

    [LibraryImport(Library)]
    private static unsafe partial int aacDecoder_Fill(nint decoder, byte** buffer, uint* size, uint* valid);

    [LibraryImport(Library)]
    private static unsafe partial int aacDecoder_DecodeFrame(nint decoder, short* pcm, int size, uint flags);

    [LibraryImport(Library)]
    private static unsafe partial StreamInfo* aacDecoder_GetStreamInfo(nint decoder);

    [LibraryImport(Library)]
    private static partial void aacDecoder_Close(nint decoder);
}
