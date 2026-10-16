using System.Runtime.InteropServices;
using System.Text;
using Vortice.Direct3D;
using Vortice.Direct3D11;

namespace Naru.Video;

/// <summary>
/// Converts a picture from Display P3 to sRGB, pixel for pixel, with a pixel shader.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>A Mac sends Display P3. Neither the video processor nor the swap chain takes P3 here, and shown as sRGB it looks washed out.</item>
/// <item>Colors past sRGB are clipped. Scaling stays with the video processor, so the shader reads each pixel once without filtering.</item>
/// <item>The shaders compile at runtime with the compiler Windows ships, d3dcompiler_47.dll.</item>
/// </list>
/// </remarks>
internal sealed unsafe partial class Gamut : IDisposable
{
    // A triangle covering the screen, and the conversion: undo the sRGB curve, change primaries, apply the curve again.
    private const string Source = """
        Texture2D<float4> picture : register(t0);

        float4 Screen(uint id : SV_VertexID) : SV_Position
        {
            float2 corner = float2((id << 1) & 2, id & 2);
            return float4(corner * float2(2, -2) + float2(-1, 1), 0, 1);
        }

        float3 Linear(float3 encoded)
        {
            return encoded <= 0.04045 ? encoded / 12.92 : pow((encoded + 0.055) / 1.055, 2.4);
        }

        float3 Encoded(float3 light)
        {
            return light <= 0.0031308 ? light * 12.92 : 1.055 * pow(light, 1 / 2.4) - 0.055;
        }

        float4 Convert(float4 position : SV_Position) : SV_Target
        {
            float3x3 displayP3ToSrgb = float3x3(
                1.2249401, -0.2249404, 0.0000000,
                -0.0420569, 1.0420571, 0.0000000,
                -0.0196376, -0.0786361, 1.0982735);
            float3 srgb = mul(displayP3ToSrgb, Linear(picture.Load(int3(position.xy, 0)).rgb));
            return float4(Encoded(saturate(srgb)), 1);
        }
        """;

    private readonly ID3D11DeviceContext context;
    private readonly ID3D11VertexShader screen;
    private readonly ID3D11PixelShader convert;

    public Gamut(Graphics graphics)
    {
        context = graphics.Context;

        using var vertex = Compile("Screen", "vs_5_0");
        using var pixel = Compile("Convert", "ps_5_0");
        screen = graphics.Device.CreateVertexShader(vertex);
        convert = graphics.Device.CreatePixelShader(pixel);
    }

    /// <summary>Draws <paramref name="source"/> into <paramref name="target"/>, both the given size, converted to sRGB.</summary>
    public void Convert(ID3D11ShaderResourceView source, ID3D11RenderTargetView target, uint width, uint height)
    {
        context.OMSetRenderTargets(target);
        context.RSSetViewport(0, 0, width, height);
        context.IASetPrimitiveTopology(PrimitiveTopology.TriangleList);
        context.VSSetShader(screen);
        context.PSSetShader(convert);
        context.PSSetShaderResource(0, source);
        context.Draw(3, 0);

        // The source is written by the video processor next frame, which a bound view would block.
        context.PSSetShaderResource(0, null!);
        context.OMSetRenderTargets((ID3D11RenderTargetView)null!);
    }

    public void Dispose()
    {
        screen.Dispose();
        convert.Dispose();
    }

    private static Blob Compile(string entry, string target)
    {
        var source = Encoding.UTF8.GetBytes(Source);

        fixed (byte* text = source)
        {
            var result = D3DCompile(text, (nuint)source.Length, null, 0, 0, entry, target, 0, 0, out var code, out var errors);

            if (result < 0)
            {
                using var message = errors == 0 ? null : new Blob(errors);
                throw new InvalidOperationException($"Cannot compile {entry}: {(message == null ? result : Marshal.PtrToStringUTF8(message.BufferPointer))}");
            }

            return new Blob(code);
        }
    }

    [LibraryImport("d3dcompiler_47.dll", StringMarshalling = StringMarshalling.Utf8)]
    private static partial int D3DCompile(byte* source, nuint size, string? name, nint defines, nint include, string entry, string target, uint flags, uint effectFlags, out nint code, out nint errors);
}
