using Microsoft.UI.Input;
using Microsoft.UI.Xaml.Controls;

namespace Naru.Session;

/// <summary>The session's screen area. WinUI shows <see cref="Cursor"/> while the pointer is over it.</summary>
/// <remarks>An element's cursor is protected in WinUI, so the screen is its own type to set it from the window.</remarks>
public sealed partial class ScreenGrid : Grid
{
    public InputCursor? Cursor
    {
        get => ProtectedCursor;
        set => ProtectedCursor = value;
    }
}
