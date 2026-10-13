using System.Numerics;
using Microsoft.UI.Composition.Interactions;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Hosting;
using Naru.Core;

namespace Naru.Session;

/// <summary>
/// Turns precision touchpad scrolling over the screen into the Mac's trackpad scrolling and its momentum.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Without this, Windows turns two-finger scrolling into small wheel steps with no start or end. An
/// InteractionTracker takes the touchpad instead and reports when fingers touch, move, and lift.</item>
/// <item>Touching is MayBegin, the first movement Began, and lifting Ended, or Cancelled if the fingers never moved.</item>
/// <item>After lifting, the tracker's own inertia continues the scroll with the system's deceleration and goes to the Mac
/// as momentum. Touching again stops it.</item>
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
    private readonly Momentum momentum;
    private readonly Func<float> pointsPerDip;
    private readonly InteractionTracker tracker;
    private Vector3 last;
    private bool interacting;
    private bool moved;
    private bool coasting;

    public Touchpad(UIElement element, Action<Phase, float, float> scroll, Momentum momentum, Func<float> pointsPerDip)
    {
        this.scroll = scroll;
        this.momentum = momentum;
        this.pointsPerDip = pointsPerDip;

        var visual = ElementCompositionPreview.GetElementVisual(element);
        tracker = InteractionTracker.CreateWithOwner(visual.Compositor, this);
        tracker.MinPosition = new Vector3(-Range, -Range, 0);
        tracker.MaxPosition = new Vector3(Range, Range, 0);

        var source = VisualInteractionSource.Create(visual);
        source.ManipulationRedirectionMode = VisualInteractionSourceRedirectionMode.CapableTouchpadOnly;
        source.PositionXSourceMode = InteractionSourceMode.EnabledWithInertia;
        source.PositionYSourceMode = InteractionSourceMode.EnabledWithInertia;
        source.ScaleSourceMode = InteractionSourceMode.Disabled;
        tracker.InteractionSources.Add(source);
    }

    public void InteractingStateEntered(InteractionTracker sender, InteractionTrackerInteractingStateEnteredArgs args)
    {
        coasting = false;
        momentum.Stop();

        interacting = true;
        moved = false;
        last = sender.Position;
        scroll(Phase.MayBegin, 0, 0);
    }

    public void ValuesChanged(InteractionTracker sender, InteractionTrackerValuesChangedArgs args)
    {
        var delta = (last - args.Position) * pointsPerDip();
        last = args.Position;

        if (delta.X == 0 && delta.Y == 0)
        {
            return;
        }

        if (interacting)
        {
            scroll(moved ? Phase.Changed : Phase.Began, delta.X, delta.Y);
            moved = true;
        }
        else if (coasting)
        {
            momentum.Move(delta.X, delta.Y);
        }
    }

    public void InertiaStateEntered(InteractionTracker sender, InteractionTrackerInertiaStateEnteredArgs args)
    {
        if (Lift())
        {
            momentum.Start(Halt);
            coasting = true;
        }
    }

    public void IdleStateEntered(InteractionTracker sender, InteractionTrackerIdleStateEnteredArgs args)
    {
        Lift();

        if (coasting)
        {
            coasting = false;
            momentum.End();
        }
    }

    public void CustomAnimationStateEntered(InteractionTracker sender, InteractionTrackerCustomAnimationStateEnteredArgs args)
    {
    }

    public void RequestIgnored(InteractionTracker sender, InteractionTrackerRequestIgnoredArgs args)
    {
    }

    // Whether the fingers scrolled before lifting
    private bool Lift()
    {
        if (!interacting)
        {
            return false;
        }

        interacting = false;
        scroll(moved ? Phase.Ended : Phase.Cancelled, 0, 0);
        return moved;
    }

    // Moving the position without velocity ends inertia.
    private void Halt()
    {
        if (coasting)
        {
            coasting = false;
            tracker.TryUpdatePositionBy(Vector3.Zero);
        }
    }
}
