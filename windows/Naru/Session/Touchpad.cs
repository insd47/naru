using System.Numerics;
using Microsoft.UI.Composition.Interactions;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Hosting;
using Naru.Core;

namespace Naru.Session;

/// <summary>
/// Turns precision touchpad scrolling over the screen into the Mac's trackpad scrolling.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Without this, Windows turns two-finger scrolling into small wheel steps with no start or end. An
/// InteractionTracker takes the touchpad instead and reports when fingers touch, move, and lift.</item>
/// <item>Touching is Began, movement is Changed, and lifting is Ended. Momentum after lifting comes from the session,
/// as macOS leaves it to the client, so Windows' own inertia is off.</item>
/// <item>The tracker's position already follows the Windows scrolling direction setting, so the content moves the way
/// the user set up Windows. The Mac's scroll delta is positive when content moves down, the opposite of the position.</item>
/// <item>Call only on the UI thread. The tracker's callbacks arrive there too.</item>
/// </list>
/// </remarks>
internal sealed partial class Touchpad : IInteractionTrackerOwner
{
    // The tracker never reaches its bounds, so it never clamps or bounces.
    private const float Range = 1e7f;

    private readonly Action<Phase, float, float> scroll;
    private readonly Func<float> pointsPerDip;
    private readonly InteractionTracker tracker;
    private Vector3 last;
    private bool interacting;

    public Touchpad(UIElement element, Action<Phase, float, float> scroll, Func<float> pointsPerDip)
    {
        this.scroll = scroll;
        this.pointsPerDip = pointsPerDip;

        var visual = ElementCompositionPreview.GetElementVisual(element);
        tracker = InteractionTracker.CreateWithOwner(visual.Compositor, this);
        tracker.MinPosition = new Vector3(-Range, -Range, 0);
        tracker.MaxPosition = new Vector3(Range, Range, 0);

        var source = VisualInteractionSource.Create(visual);
        source.ManipulationRedirectionMode = VisualInteractionSourceRedirectionMode.CapableTouchpadOnly;
        source.PositionXSourceMode = InteractionSourceMode.EnabledWithoutInertia;
        source.PositionYSourceMode = InteractionSourceMode.EnabledWithoutInertia;
        source.ScaleSourceMode = InteractionSourceMode.Disabled;
        tracker.InteractionSources.Add(source);
    }

    public void InteractingStateEntered(InteractionTracker sender, InteractionTrackerInteractingStateEnteredArgs args)
    {
        interacting = true;
        last = sender.Position;
        scroll(Phase.Began, 0, 0);
    }

    public void ValuesChanged(InteractionTracker sender, InteractionTrackerValuesChangedArgs args)
    {
        if (!interacting)
        {
            return;
        }

        var delta = (args.Position - last) * pointsPerDip();
        last = args.Position;
        scroll(Phase.Changed, -delta.X, -delta.Y);
    }

    public void IdleStateEntered(InteractionTracker sender, InteractionTrackerIdleStateEnteredArgs args) => End();

    public void InertiaStateEntered(InteractionTracker sender, InteractionTrackerInertiaStateEnteredArgs args) => End();

    public void CustomAnimationStateEntered(InteractionTracker sender, InteractionTrackerCustomAnimationStateEnteredArgs args)
    {
    }

    public void RequestIgnored(InteractionTracker sender, InteractionTrackerRequestIgnoredArgs args)
    {
    }

    private void End()
    {
        if (interacting)
        {
            interacting = false;
            scroll(Phase.Ended, 0, 0);
        }
    }
}
