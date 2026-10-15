using Microsoft.UI.Input;
using Naru.Core;
using Windows.Foundation;
using FingerTouchpad = Naru.Core.Touchpad;

namespace Naru.Session;

/// <summary>
/// Turns fingers on the screen into the Mac's trackpad gestures, with the whole screen as one trackpad.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>The core's touchpad recognizes the gestures. This moves the Mac's cursor by them, clamped to the screen, and
/// draws it there, since touch has no system pointer. The cursor starts from wherever a mouse or pen last put it.</item>
/// <item>A two-finger scroll continues after the fingers lift with a GestureRecognizer's inertia, the system's deceleration
/// for touch, and goes to the Mac as momentum.</item>
/// <item>Call only on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed class TouchScreen
{
    private readonly Connection connection;
    private readonly Momentum momentum;
    private readonly TouchCursor cursor;
    private readonly FingerTouchpad fingers = new();
    private readonly GestureRecognizer recognizer = new()
    {
        GestureSettings = GestureSettings.ManipulationTranslateX | GestureSettings.ManipulationTranslateY | GestureSettings.ManipulationTranslateInertia,
    };

    // The recognizer rejects points of a pointer it has not seen go down.
    private readonly HashSet<uint> down = [];

    // Where the Mac's cursor is in layout pixels, whether it is drawn, and the buttons finger taps hold
    private Point position;
    private bool drawn;
    private byte buttons;

    // The scroll ended, so the recognizer's inertia continues it in the Mac's points per DIP
    private bool flinging;
    private float pointsPerDip;
    private bool inertia;

    public TouchScreen(Connection connection, Momentum momentum, TouchCursor cursor)
    {
        this.connection = connection;
        this.momentum = momentum;
        this.cursor = cursor;
        // Updates after inertia starts are the deceleration; the ones before are the fingers, which the touchpad already sent.
        recognizer.ManipulationStarted += (_, _) => inertia = false;
        recognizer.ManipulationInertiaStarting += (_, _) => inertia = true;
        recognizer.ManipulationUpdated += (_, args) =>
        {
            if (flinging && inertia)
            {
                momentum.Move((float)args.Delta.Translation.X * pointsPerDip, (float)args.Delta.Translation.Y * pointsPerDip);
            }
        };
        recognizer.ManipulationCompleted += (_, _) =>
        {
            inertia = false;

            if (flinging)
            {
                flinging = false;
                momentum.End();
            }
        };
    }

    /// <summary>One finger, at a point of the screen area with the points it passed since the last event.</summary>
    public void Touch(PointerPoint point, IList<PointerPoint> passed, TouchPhase phase, Placement place)
    {
        var at = place.Unclamped(point.Position);

        foreach (var gesture in fingers.Touch(point.PointerId, phase, (float)at.X, (float)at.Y))
        {
            Apply(gesture, place);
        }

        // The touchpad goes first, so a scroll has ended before the recognizer starts its inertia.
        switch (phase)
        {
            case TouchPhase.Began when down.Add(point.PointerId):
                recognizer.ProcessDownEvent(point);
                break;
            case TouchPhase.Moved when down.Contains(point.PointerId):
                recognizer.ProcessMoveEvents(passed);
                break;
            case TouchPhase.Ended or TouchPhase.Cancelled when down.Remove(point.PointerId):
                recognizer.ProcessUpEvent(point);
                break;
        }
    }

    /// <summary>A mouse or pen moved the Mac's cursor here, in layout pixels, and shows the system pointer instead.</summary>
    public void Follow(Point mac)
    {
        position = mac;
        drawn = false;
        cursor.Hide();
    }

    /// <summary>Draws the cursor again after the picture moved.</summary>
    public void Place(Placement place)
    {
        if (drawn)
        {
            cursor.Place(place.Screen(position), place.PixelsPerPoint / place.Scale);
        }
    }

    private void Apply(Gesture gesture, Placement place)
    {
        switch (gesture)
        {
            case Gesture.Move move:
                position = new Point(
                    Math.Clamp(position.X + move.Dx, 0, place.Layout.PixelWidth - 1),
                    Math.Clamp(position.Y + move.Dy, 0, place.Layout.PixelHeight - 1));
                Send();
                break;
            case Gesture.Buttons pressed:
                buttons = pressed.ButtonsValue;
                Send();
                break;
            case Gesture.Scroll scroll:
                var pointsPerPixel = (float)place.PointsPerPixel;
                connection.Scroll(scroll.Phase, scroll.Dx * pointsPerPixel, scroll.Dy * pointsPerPixel);

                if (scroll.Phase == Phase.Ended)
                {
                    momentum.Start(Halt);
                    flinging = true;
                    pointsPerDip = (float)place.PointsPerDip;
                }

                break;
            case Gesture.Magnify magnify:
                connection.Magnify(magnify.Phase, magnify.Scale);
                break;
        }

        void Send()
        {
            drawn = true;
            Place(place);
            connection.Pointer(buttons, (ushort)Math.Round(position.X), (ushort)Math.Round(position.Y));
        }
    }

    private void Halt()
    {
        if (flinging)
        {
            flinging = false;
            recognizer.CompleteGesture();
        }
    }
}
