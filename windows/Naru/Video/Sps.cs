namespace Naru.Video;

/// <summary>
/// Rewrites an H.264 SPS so the decoder outputs each picture as soon as it is decoded.
/// </summary>
/// <remarks>
/// The server's stream has no B-frames but does not say so: its SPS lacks the VUI bitstream restriction and declares
/// 11 reference frames. Without the restriction, Windows' decoder holds each picture until the next one arrives in
/// case it must reorder them, which adds a frame of latency. Declaring <c>max_num_reorder_frames = 0</c> removes the
/// wait. Every other field is kept as it was.
/// </remarks>
internal static class Sps
{
    private static readonly HashSet<uint> HighProfiles = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

    /// <summary>The SPS NAL (without a start code) declaring no reordering, or the original if it cannot be parsed.</summary>
    public static byte[] WithoutReordering(byte[] nal)
    {
        try
        {
            return Rewrite(nal);
        }
        catch (FormatException)
        {
            return nal;
        }
    }

    private static byte[] Rewrite(byte[] nal)
    {
        var reader = new BitReader(Unescape(nal));
        reader.Bits(8);
        var profile = reader.Bits(8);
        reader.Bits(16);
        reader.Ue();
        uint chromaFormat = 1;

        if (HighProfiles.Contains(profile))
        {
            chromaFormat = reader.Ue();

            if (chromaFormat == 3)
            {
                reader.Bits(1);
            }

            reader.Ue();
            reader.Ue();
            reader.Bits(1);

            if (reader.Flag())
            {
                for (var list = 0; list < (chromaFormat == 3 ? 12 : 8); list++)
                {
                    if (reader.Flag())
                    {
                        SkipScalingList(reader, list < 6 ? 16 : 64);
                    }
                }
            }
        }

        reader.Ue();

        switch (reader.Ue())
        {
            case 0:
                reader.Ue();
                break;
            case 1:
                reader.Bits(1);
                reader.Se();
                reader.Se();

                for (var cycle = reader.Ue(); cycle > 0; cycle--)
                {
                    reader.Se();
                }

                break;
        }

        var maxRefFrames = reader.Ue();
        reader.Bits(1);
        reader.Ue();
        reader.Ue();

        if (!reader.Flag())
        {
            reader.Bits(1);
        }

        reader.Bits(1);

        if (reader.Flag())
        {
            reader.Ue();
            reader.Ue();
            reader.Ue();
            reader.Ue();
        }

        // Everything up to here is copied unchanged. The VUI is rewritten up to its bitstream restriction.
        var vuiAt = reader.Position;
        var writer = new BitWriter();
        writer.Copy(reader.Data, 0, vuiAt);
        writer.Bit(true);

        Restriction? existing = null;

        if (reader.Flag())
        {
            CopyVuiUntilRestriction(reader, writer);

            if (reader.Flag())
            {
                existing = new Restriction(reader.Flag(), reader.Ue(), reader.Ue(), reader.Ue(), reader.Ue(), reader.Ue(), reader.Ue());

                if (existing.Value.MaxNumReorderFrames == 0)
                {
                    return nal;
                }
            }
        }
        else
        {
            // No VUI yet: every presence flag before the restriction is off.
            for (var flag = 0; flag < 8; flag++)
            {
                writer.Bit(false);
            }
        }

        var restriction = existing is { } found
            ? found with { MaxNumReorderFrames = 0, MaxDecFrameBuffering = Math.Max(found.MaxDecFrameBuffering, 1) }
            : new Restriction(true, 2, 1, 16, 16, 0, Math.Max(maxRefFrames, 1));

        writer.Bit(true);
        writer.Bit(restriction.MotionVectorsOverPicBoundaries);
        writer.Ue(restriction.MaxBytesPerPicDenom);
        writer.Ue(restriction.MaxBitsPerMbDenom);
        writer.Ue(restriction.Log2MaxMvLengthHorizontal);
        writer.Ue(restriction.Log2MaxMvLengthVertical);
        writer.Ue(restriction.MaxNumReorderFrames);
        writer.Ue(restriction.MaxDecFrameBuffering);
        writer.Trailing();

        return Escape(writer.ToArray());
    }

    // Copies the VUI fields before bitstream_restriction_flag, leaving the reader on that flag.
    private static void CopyVuiUntilRestriction(BitReader reader, BitWriter writer)
    {
        var start = reader.Position;

        if (reader.Flag() && reader.Bits(8) == 255)
        {
            reader.Bits(32);
        }

        if (reader.Flag())
        {
            reader.Bits(1);
        }

        if (reader.Flag())
        {
            reader.Bits(4);

            if (reader.Flag())
            {
                reader.Bits(24);
            }
        }

        if (reader.Flag())
        {
            reader.Ue();
            reader.Ue();
        }

        if (reader.Flag())
        {
            reader.Bits(32);
            reader.Bits(32);
            reader.Bits(1);
        }

        var nal = reader.Flag();

        if (nal)
        {
            SkipHrd(reader);
        }

        var vcl = reader.Flag();

        if (vcl)
        {
            SkipHrd(reader);
        }

        if (nal || vcl)
        {
            reader.Bits(1);
        }

        reader.Bits(1);
        writer.Copy(reader.Data, start, reader.Position - start);
    }

    private static void SkipHrd(BitReader reader)
    {
        var count = reader.Ue() + 1;
        reader.Bits(8);

        for (var index = 0; index < count; index++)
        {
            reader.Ue();
            reader.Ue();
            reader.Bits(1);
        }

        reader.Bits(20);
    }

    private static void SkipScalingList(BitReader reader, int size)
    {
        var last = 8;
        var next = 8;

        for (var index = 0; index < size; index++)
        {
            if (next != 0)
            {
                next = (last + reader.Se() + 256) % 256;
            }

            last = next == 0 ? last : next;
        }
    }

    // Removes emulation prevention bytes (the 03 in 00 00 03).
    private static byte[] Unescape(byte[] nal)
    {
        var output = new List<byte>(nal.Length);
        var zeros = 0;

        foreach (var value in nal)
        {
            if (zeros >= 2 && value == 3)
            {
                zeros = 0;
                continue;
            }

            zeros = value == 0 ? zeros + 1 : 0;
            output.Add(value);
        }

        return [.. output];
    }

    // Inserts emulation prevention bytes where 00 00 is followed by a byte of 3 or less.
    private static byte[] Escape(byte[] rbsp)
    {
        var output = new List<byte>(rbsp.Length + 4);
        var zeros = 0;

        foreach (var value in rbsp)
        {
            if (zeros >= 2 && value <= 3)
            {
                output.Add(3);
                zeros = 0;
            }

            zeros = value == 0 ? zeros + 1 : 0;
            output.Add(value);
        }

        return [.. output];
    }

    // The VUI bitstream restriction fields
    private readonly record struct Restriction(
        bool MotionVectorsOverPicBoundaries,
        uint MaxBytesPerPicDenom,
        uint MaxBitsPerMbDenom,
        uint Log2MaxMvLengthHorizontal,
        uint Log2MaxMvLengthVertical,
        uint MaxNumReorderFrames,
        uint MaxDecFrameBuffering);

    private sealed class BitReader(byte[] data)
    {
        public byte[] Data { get; } = data;
        public int Position { get; private set; }

        public bool Flag() => Bits(1) == 1;

        public uint Bits(int count)
        {
            uint value = 0;

            for (var index = 0; index < count; index++)
            {
                if (Position >= Data.Length * 8)
                {
                    throw new FormatException("SPS ended early");
                }

                value = (value << 1) | (uint)((Data[Position / 8] >> (7 - Position % 8)) & 1);
                Position++;
            }

            return value;
        }

        public uint Ue()
        {
            var leading = 0;

            while (Bits(1) == 0)
            {
                if (++leading > 31)
                {
                    throw new FormatException("Exp-Golomb code too long");
                }
            }

            return (uint)((1UL << leading) - 1 + Bits(leading));
        }

        public int Se()
        {
            var code = Ue();
            return code % 2 == 1 ? (int)((code + 1) / 2) : -(int)(code / 2);
        }
    }

    private sealed class BitWriter
    {
        private readonly List<byte> bytes = [];
        private int bit;

        public void Bit(bool value)
        {
            if (bit == 0)
            {
                bytes.Add(0);
            }

            if (value)
            {
                bytes[^1] |= (byte)(0x80 >> bit);
            }

            bit = (bit + 1) % 8;
        }

        public void Ue(uint value)
        {
            var code = (ulong)value + 1;
            var length = 64 - System.Numerics.BitOperations.LeadingZeroCount(code);

            for (var index = 0; index < length - 1; index++)
            {
                Bit(false);
            }

            for (var index = length - 1; index >= 0; index--)
            {
                Bit(((code >> index) & 1) == 1);
            }
        }

        public void Copy(byte[] source, int from, int count)
        {
            for (var index = from; index < from + count; index++)
            {
                Bit(((source[index / 8] >> (7 - index % 8)) & 1) == 1);
            }
        }

        // rbsp_stop_one_bit, then zeros to the byte boundary
        public void Trailing()
        {
            Bit(true);

            while (bit != 0)
            {
                Bit(false);
            }
        }

        public byte[] ToArray() => [.. bytes];
    }
}
