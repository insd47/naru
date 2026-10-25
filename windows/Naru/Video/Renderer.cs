using System.Numerics;
using System.Runtime.InteropServices;
using Microsoft.UI.Xaml.Controls;
using Vortice.Direct3D11;
using Vortice.DXGI;
using Vortice.Mathematics;

namespace Naru.Video;

/// <summary>
/// Presents decoded pictures in a SwapChainPanel, scaled to fit with the aspect ratio kept.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The swap chain matches the panel's physical pixels, and its transform undoes the panel's scale, so nothing is resampled twice.</item>
/// <item>The video processor converts NV12 (H.264) or AYUV (HEVC 4:4:4) to BGRA and scales in one pass on the GPU.</item>
/// <item>Display P3 is drawn into a texture the size of the screen first, and <see cref="Gamut"/> converts it to sRGB on the way to the swap chain.</item>
/// <item>When the panel changes size, the buffers keep theirs and the transform stretches the last picture over the panel, until the next picture is drawn at the new size. Screen Sharing keeps the last picture the same way while the Mac's display changes size.</item>
/// <item>Call only on the video thread, except <see cref="Attach"/>, which runs on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed class Renderer : IDisposable
{
    private const Format BackBufferFormat = Format.B8G8R8A8_UNorm;

    private readonly Graphics graphics;
    private readonly ID3D11VideoDevice videoDevice;
    private readonly ID3D11VideoContext1 videoContext;
    private readonly IDXGISwapChain2 swapChain;
    private readonly Gamut gamut;

    private ID3D11VideoProcessorEnumerator? enumerator;
    private ID3D11VideoProcessor? processor;
    private (uint Width, uint Height, uint OutputWidth, uint OutputHeight) processorSize;
    private readonly Dictionary<(nint, uint), ID3D11VideoProcessorInputView> inputs = [];
    // The panel's size in physical pixels and its scale, and the buffers' size, which follows at the next drawing
    private uint width;
    private uint height;
    private float scale;
    private (uint Width, uint Height) buffers;
    // Where the last picture went in the buffers, or null when they hold none
    private (int Left, int Top, int Width, int Height)? shown;

    // The scaled picture before its colors are converted, the size of the swap chain
    private ID3D11Texture2D? scaled;
    private ID3D11ShaderResourceView? scaledView;

    public Renderer(Graphics graphics, uint width, uint height, float scale)
    {
        this.graphics = graphics;
        this.width = Math.Max(1, width);
        this.height = Math.Max(1, height);
        videoDevice = graphics.Device.QueryInterface<ID3D11VideoDevice>();
        videoContext = graphics.Context.QueryInterface<ID3D11VideoContext1>();
        gamut = new Gamut(graphics);

        using var device = graphics.Device.QueryInterface<IDXGIDevice>();
        using var adapter = device.GetAdapter();
        using var factory = adapter.GetParent<IDXGIFactory2>();
        var description = new SwapChainDescription1
        {
            Width = this.width,
            Height = this.height,
            Format = BackBufferFormat,
            SampleDescription = new SampleDescription(1, 0),
            BufferUsage = Usage.RenderTargetOutput,
            BufferCount = 2,
            Scaling = Scaling.Stretch,
            SwapEffect = SwapEffect.FlipSequential,
            AlphaMode = AlphaMode.Ignore,
        };

        using var created = factory.CreateSwapChainForComposition(graphics.Device, description);
        swapChain = created.QueryInterface<IDXGISwapChain2>();
        buffers = (this.width, this.height);
        this.scale = scale;
        Transform();
        Clear();
    }

    /// <summary>Hands the swap chain to the panel. Must run on the UI thread.</summary>
    public unsafe void Attach(SwapChainPanel panel)
    {
        var unknown = ((WinRT.IWinRTObject)panel).NativeObject.ThisPtr;
        var iid = new Guid("63aad0b8-7c24-40ff-85a8-640d944cc325");
        Marshal.ThrowExceptionForHR(Marshal.QueryInterface(unknown, in iid, out var native));

        try
        {
            // ISwapChainPanelNative::SetSwapChain is the first method after IUnknown's three.
            var setSwapChain = (delegate* unmanaged[Stdcall]<nint, nint, int>)(*(void***)native)[3];
            Marshal.ThrowExceptionForHR(setSwapChain(native, swapChain.NativePointer));
        }
        finally
        {
            Marshal.Release(native);
        }
    }

    public void Resize(uint nextWidth, uint nextHeight, float nextScale)
    {
        width = Math.Max(1, nextWidth);
        height = Math.Max(1, nextHeight);
        scale = nextScale;
        Transform();
    }

    /// <summary>Draws a slice of a texture, cropped to the visible size the Mac reports, placed by <see cref="Fit"/> with black around it.</summary>
    public void Render(ID3D11Texture2D source, uint slice, Colors colors, uint visibleWidth, uint visibleHeight)
    {
        Match();
        var texture = source.Description;
        var sourceWidth = Math.Min(visibleWidth == 0 ? texture.Width : visibleWidth, texture.Width);
        var sourceHeight = Math.Min(visibleHeight == 0 ? texture.Height : visibleHeight, texture.Height);
        Prepare(texture.Width, texture.Height);

        var key = (source.NativePointer, slice);

        if (!inputs.TryGetValue(key, out var input))
        {
            input = videoDevice.CreateVideoProcessorInputView(source, enumerator!, new VideoProcessorInputViewDescription
            {
                FourCC = 0,
                ViewDimension = VideoProcessorInputViewDimension.Texture2D,
                Texture2D = new Texture2DVideoProcessorInputView { MipSlice = 0, ArraySlice = slice },
            });
            inputs[key] = input;
        }

        using var backBuffer = swapChain.GetBuffer<ID3D11Texture2D>(0);
        var drawn = colors.DisplayP3 ? Scaled() : backBuffer;
        using var output = videoDevice.CreateVideoProcessorOutputView(drawn, enumerator!, new VideoProcessorOutputViewDescription
        {
            ViewDimension = VideoProcessorOutputViewDimension.Texture2D,
        });

        var fit = Fit.Of(width, height, sourceWidth, sourceHeight);
        var drawWidth = (int)Math.Round(sourceWidth * fit.Scale);
        var drawHeight = (int)Math.Round(sourceHeight * fit.Scale);
        var left = (int)fit.Left;
        var top = (int)fit.Top;

        videoContext.VideoProcessorSetStreamColorSpace1(processor!, 0, colors.StudioRange ? ColorSpaceType.YcbcrStudioG22LeftP709 : ColorSpaceType.YcbcrFullG22LeftP709);
        videoContext.VideoProcessorSetOutputColorSpace1(processor!, ColorSpaceType.RgbFullG22NoneP709);
        videoContext.VideoProcessorSetStreamFrameFormat(processor!, 0, VideoFrameFormat.Progressive);
        videoContext.VideoProcessorSetStreamAutoProcessingMode(processor!, 0, false);
        videoContext.VideoProcessorSetStreamSourceRect(processor!, 0, true, new Vortice.RawRect(0, 0, (int)sourceWidth, (int)sourceHeight));
        videoContext.VideoProcessorSetStreamDestRect(processor!, 0, true, new Vortice.RawRect(left, top, left + drawWidth, top + drawHeight));
        videoContext.VideoProcessorSetOutputTargetRect(processor!, true, new Vortice.RawRect(0, 0, (int)width, (int)height));

        var stream = new VideoProcessorStream { Enable = true, InputSurface = input };
        videoContext.VideoProcessorBlt(processor!, output, 0, 1, [stream]).CheckError();

        if (colors.DisplayP3)
        {
            using var target = graphics.Device.CreateRenderTargetView(backBuffer);
            gamut.Convert(scaledView!, target, width, height);
        }

        swapChain.Present(0, PresentFlags.None);
        shown = (left, top, drawWidth, drawHeight);
    }

    /// <summary>
    /// The picture last presented as BGRA rows without padding, or null when none is on screen. A flip model swap
    /// chain lets its buffers past the first be read, and the last one holds what was presented last.
    /// </summary>
    public (byte[] Pixels, int Width, int Height)? Picture()
    {
        if (shown is not { } picture || picture.Width <= 0 || picture.Height <= 0)
        {
            return null;
        }

        using var last = swapChain.GetBuffer<ID3D11Texture2D>(1);
        using var staging = graphics.Device.CreateTexture2D(new Texture2DDescription
        {
            Width = (uint)picture.Width,
            Height = (uint)picture.Height,
            MipLevels = 1,
            ArraySize = 1,
            Format = BackBufferFormat,
            SampleDescription = new SampleDescription(1, 0),
            Usage = ResourceUsage.Staging,
            CPUAccessFlags = CpuAccessFlags.Read,
        });

        var region = new Box(picture.Left, picture.Top, 0, picture.Left + picture.Width, picture.Top + picture.Height, 1);
        graphics.Context.CopySubresourceRegion(staging, 0, 0, 0, 0, last, 0, region);
        var mapped = graphics.Context.Map(staging, 0, MapMode.Read);

        try
        {
            var rowBytes = picture.Width * 4;
            var pixels = new byte[rowBytes * picture.Height];

            for (var y = 0; y < picture.Height; y++)
            {
                Marshal.Copy(mapped.DataPointer + y * (nint)mapped.RowPitch, pixels, y * rowBytes, rowBytes);
            }

            return (pixels, picture.Width, picture.Height);
        }
        finally
        {
            graphics.Context.Unmap(staging, 0);
        }
    }

    /// <summary>Fills the screen with black, before the first picture or after the video stops.</summary>
    public void Clear()
    {
        Match();
        using var backBuffer = swapChain.GetBuffer<ID3D11Texture2D>(0);
        using var target = graphics.Device.CreateRenderTargetView(backBuffer);
        graphics.Context.ClearRenderTargetView(target, new Color4(0, 0, 0, 1));
        swapChain.Present(0, PresentFlags.None);
        shown = null;
    }

    public void Dispose()
    {
        Release();
        ReleaseScaled();
        gamut.Dispose();
        swapChain.Dispose();
        videoContext.Dispose();
        videoDevice.Dispose();
    }

    // Brings the buffers to the panel's size before drawing.
    private void Match()
    {
        if (buffers != (width, height))
        {
            ReleaseScaled();
            swapChain.ResizeBuffers(0, width, height, Format.Unknown, SwapChainFlags.None).CheckError();
            buffers = (width, height);
            shown = null;
            Transform();
        }
    }

    // The panel is scaled by the monitor's scale. The inverse transform maps one swap chain pixel to one screen pixel,
    // and stretches buffers of another size over the panel.
    private void Transform()
    {
        swapChain.MatrixTransform = Matrix3x2.CreateScale((float)width / buffers.Width / scale, (float)height / buffers.Height / scale);
    }

    // The processor depends on the input and output sizes, so it is recreated only when either changes.
    private void Prepare(uint inputWidth, uint inputHeight)
    {
        var size = (inputWidth, inputHeight, width, height);

        if (processor != null && processorSize == size)
        {
            return;
        }

        Release();
        enumerator = videoDevice.CreateVideoProcessorEnumerator(new VideoProcessorContentDescription
        {
            InputFrameFormat = VideoFrameFormat.Progressive,
            InputWidth = inputWidth,
            InputHeight = inputHeight,
            OutputWidth = width,
            OutputHeight = height,
            Usage = VideoUsage.OptimalSpeed,
        });
        processor = videoDevice.CreateVideoProcessor(enumerator, 0);
        processorSize = size;
    }

    /// <summary>Lets go of the textures drawn so far, before their owner releases them. A view keeps its texture alive, and a new texture may reuse an old one's address.</summary>
    public void Forget()
    {
        foreach (var input in inputs.Values)
        {
            input.Dispose();
        }

        inputs.Clear();
    }

    private ID3D11Texture2D Scaled()
    {
        if (scaled == null)
        {
            scaled = graphics.Device.CreateTexture2D(new Texture2DDescription
            {
                Width = width,
                Height = height,
                MipLevels = 1,
                ArraySize = 1,
                Format = BackBufferFormat,
                SampleDescription = new SampleDescription(1, 0),
                BindFlags = BindFlags.RenderTarget | BindFlags.ShaderResource,
            });
            scaledView = graphics.Device.CreateShaderResourceView(scaled);
        }

        return scaled;
    }

    private void ReleaseScaled()
    {
        scaledView?.Dispose();
        scaledView = null;
        scaled?.Dispose();
        scaled = null;
    }

    private void Release()
    {
        Forget();
        processor?.Dispose();
        processor = null;
        enumerator?.Dispose();
        enumerator = null;
    }
}
