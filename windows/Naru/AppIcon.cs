using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;

namespace Naru;

/// <summary>
/// The title bar icon, drawn in the variant for the current theme.
/// </summary>
/// <remarks>
/// The .ico carries only the light-theme art, so title bars use the unplated PNGs instead: <c>altform-unplated</c> for
/// dark and <c>altform-lightunplated</c> for light, at the target size closest to 16 pixels times the scale.
/// </remarks>
internal static class AppIcon
{
    private static readonly int[] Sizes = [16, 20, 24, 32, 40, 48];

    /// <summary>Keeps the title bar's icon matching the theme and the monitor scale.</summary>
    public static void Apply(TitleBar bar) =>
        Follow(bar, () => bar.IconSource = new ImageIconSource { ImageSource = Source(bar.ActualTheme, bar.XamlRoot?.RasterizationScale ?? 1) });

    /// <summary>Keeps an image in a custom title bar matching the theme and the monitor scale.</summary>
    public static void Apply(Image image) =>
        Follow(image, () => image.Source = Source(image.ActualTheme, image.XamlRoot?.RasterizationScale ?? 1));

    private static void Follow(FrameworkElement element, Action update)
    {
        element.ActualThemeChanged += (_, _) => update();
        element.Loaded += (_, _) =>
        {
            update();
            element.XamlRoot.Changed += (_, _) => update();
        };
        update();
    }

    private static BitmapImage Source(ElementTheme theme, double scale)
    {
        var wanted = 16 * scale;
        var size = Sizes.FirstOrDefault(size => size >= wanted, Sizes[^1]);
        var variant = theme == ElementTheme.Light ? "lightunplated" : "unplated";
        return new BitmapImage(new Uri($"ms-appx:///Assets/Square44x44Logo.targetsize-{size}_altform-{variant}.png"));
    }
}
