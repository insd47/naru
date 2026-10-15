using System.Runtime.InteropServices;
using Naru.Core;
using Vortice.Multimedia;
using Vortice.XAudio2;

namespace Naru.Audio;

/// <summary>
/// Plays the Mac's audio: decodes AAC-ELD with <see cref="Aac"/> and queues the PCM to an XAudio2 voice.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Call only from the session's event thread. Audio plays as it arrives, without syncing to video.</item>
/// <item>At most <see cref="MaxQueued"/> frames wait to play. Frames beyond that are dropped so latency cannot build up.</item>
/// <item>XAudio2 reads queued buffers in place, so PCM is copied into a ring of native slots. A slot is reused only after it played.</item>
/// <item>Create a new one for each AudioSpecificConfig.</item>
/// </list>
/// </remarks>
internal sealed unsafe class Speaker : IDisposable
{
    // Frames waiting to play, 10 ms each
    private const int MaxQueued = 6;

    // More slots than frames that may be queued, so the slot written next has always finished playing.
    private const int Slots = MaxQueued + 2;
    private const int SlotBytes = Aac.MaxSamples * sizeof(short);

    private readonly Aac decoder;
    private readonly IXAudio2 engine;
    private readonly IXAudio2MasteringVoice master;
    private IXAudio2SourceVoice? voice;
    private readonly byte* slots;
    private int next;

    public Speaker(byte[] config)
    {
        decoder = new Aac(config);
        engine = XAudio2.XAudio2Create();
        master = engine.CreateMasteringVoice();
        slots = (byte*)NativeMemory.Alloc(Slots * SlotBytes);
    }

    public void Play(AudioFrame frame)
    {
        var pcm = decoder.Decode(frame.Data);

        if (pcm.IsEmpty)
        {
            return;
        }

        // The voice takes its format from the first decoded frame.
        if (voice == null)
        {
            voice = engine.CreateSourceVoice(new WaveFormat(decoder.SampleRate, 16, decoder.Channels));
            voice.Start();
            Log.Info($"Audio: {decoder.SampleRate} Hz, {decoder.Channels} channels");
        }

        if (voice.State.BuffersQueued >= MaxQueued)
        {
            return;
        }

        var bytes = MemoryMarshal.AsBytes(pcm);
        var slot = slots + next * SlotBytes;
        bytes.CopyTo(new Span<byte>(slot, SlotBytes));
        next = (next + 1) % Slots;
        voice.SubmitSourceBuffer(new AudioBuffer((nint)slot, (uint)bytes.Length));
    }

    public void Dispose()
    {
        // Destroying the voice stops it and waits until it no longer reads the slots.
        voice?.DestroyVoice();
        voice?.Dispose();
        master.DestroyVoice();
        master.Dispose();
        engine.Dispose();
        NativeMemory.Free(slots);
        decoder.Dispose();
    }
}
