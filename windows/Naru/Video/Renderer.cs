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
/// <item>The video processor converts NV12 to BGRA and scales in one pass on the GPU.</item>
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

    private ID3D11VideoProcessorEnumerator? enumerator;
    private ID3D11VideoProcessor? processor;
    private (uint Width, uint Height, uint OutputWidth, uint OutputHeight) processorSize;
    private readonly Dictionary<(nint, uint), ID3D11VideoProcessorInputView> inputs = [];
    private uint width;
    private uint height;

    public Renderer(Graphics graphics, uint width, uint height, float scale)
    {
        this.graphics = graphics;
        this.width = Math.Max(1, width);
        this.height = Math.Max(1, height);
        videoDevice = graphics.Device.QueryInterface<ID3D11VideoDevice>();
        videoContext = graphics.Context.QueryInterface<ID3D11VideoContext1>();

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
        Scale(scale);
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

    public void Resize(uint nextWidth, uint nextHeight, float scale)
    {
        nextWidth = Math.Max(1, nextWidth);
        nextHeight = Math.Max(1, nextHeight);

        if (nextWidth != width || nextHeight != height)
        {
            width = nextWidth;
            height = nextHeight;
            swapChain.ResizeBuffers(0, width, height, Format.Unknown, SwapChainFlags.None).CheckError();
        }

        Scale(scale);
    }

    /// <summary>Draws a picture, cropped to the visible size the Mac reports, placed by <see cref="Fit"/> with black around it.</summary>
    public void Render(Picture picture, uint visibleWidth, uint visibleHeight)
    {
        var texture = picture.Texture.Description;
        var sourceWidth = Math.Min(visibleWidth == 0 ? texture.Width : visibleWidth, texture.Width);
        var sourceHeight = Math.Min(visibleHeight == 0 ? texture.Height : visibleHeight, texture.Height);
        Prepare(texture.Width, texture.Height);

        var key = (picture.Texture.NativePointer, picture.Slice);

        if (!inputs.TryGetValue(key, out var input))
        {
            input = videoDevice.CreateVideoProcessorInputView(picture.Texture, enumerator!, new VideoProcessorInputViewDescription
            {
                FourCC = 0,
                ViewDimension = VideoProcessorInputViewDimension.Texture2D,
                Texture2D = new Texture2DVideoProcessorInputView { MipSlice = 0, ArraySlice = picture.Slice },
            });
            inputs[key] = input;
        }

        using var backBuffer = swapChain.GetBuffer<ID3D11Texture2D>(0);
        using var output = videoDevice.CreateVideoProcessorOutputView(backBuffer, enumerator!, new VideoProcessorOutputViewDescription
        {
            ViewDimension = VideoProcessorOutputViewDimension.Texture2D,
        });

        var fit = Fit.Of(width, height, sourceWidth, sourceHeight);
        var drawWidth = (int)Math.Round(sourceWidth * fit.Scale);
        var drawHeight = (int)Math.Round(sourceHeight * fit.Scale);
        var left = (int)fit.Left;
        var top = (int)fit.Top;

        videoContext.VideoProcessorSetStreamColorSpace1(processor!, 0, picture.StudioRange ? ColorSpaceType.YcbcrStudioG22LeftP709 : ColorSpaceType.YcbcrFullG22LeftP709);
        videoContext.VideoProcessorSetOutputColorSpace1(processor!, ColorSpaceType.RgbFullG22NoneP709);
        videoContext.VideoProcessorSetStreamFrameFormat(processor!, 0, VideoFrameFormat.Progressive);
        videoContext.VideoProcessorSetStreamAutoProcessingMode(processor!, 0, false);
        videoContext.VideoProcessorSetStreamSourceRect(processor!, 0, true, new Vortice.RawRect(0, 0, (int)sourceWidth, (int)sourceHeight));
        videoContext.VideoProcessorSetStreamDestRect(processor!, 0, true, new Vortice.RawRect(left, top, left + drawWidth, top + drawHeight));
        videoContext.VideoProcessorSetOutputTargetRect(processor!, true, new Vortice.RawRect(0, 0, (int)width, (int)height));

        var stream = new VideoProcessorStream { Enable = true, InputSurface = input };
        videoContext.VideoProcessorBlt(processor!, output, 0, 1, [stream]).CheckError();
        swapChain.Present(0, PresentFlags.None);
    }

    /// <summary>Fills the screen with black, before the first picture or after the video stops.</summary>
    public void Clear()
    {
        using var backBuffer = swapChain.GetBuffer<ID3D11Texture2D>(0);
        using var target = graphics.Device.CreateRenderTargetView(backBuffer);
        graphics.Context.ClearRenderTargetView(target, new Color4(0, 0, 0, 1));
        swapChain.Present(0, PresentFlags.None);
    }

    public void Dispose()
    {
        Release();
        swapChain.Dispose();
        videoContext.Dispose();
        videoDevice.Dispose();
    }

    // The panel is scaled by the monitor's scale. The inverse transform maps one swap chain pixel to one screen pixel.
    private void Scale(float scale)
    {
        swapChain.MatrixTransform = Matrix3x2.CreateScale(1 / scale);
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

    private void Release()
    {
        foreach (var input in inputs.Values)
        {
            input.Dispose();
        }

        inputs.Clear();
        processor?.Dispose();
        processor = null;
        enumerator?.Dispose();
        enumerator = null;
    }
}
