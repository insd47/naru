using Naru.Core;

namespace Naru.Session;

/// <summary>
/// Sends the momentum of one scroll at a time to the Mac.
/// </summary>
/// <remarks>
/// <list type="bullet">
/// <item>Windows reports nothing after a scroll ends; its scrolling views decelerate it themselves. The touchpad's
/// InteractionTracker and the touch screen's GestureRecognizer do the same here and report each step.</item>
/// <item>The Mac hears Began with the first movement, and Ended once the deceleration rests or new input stops it.</item>
/// <item>Call only on the UI thread.</item>
/// </list>
/// </remarks>
internal sealed class Momentum(Connection connection)
{
    private Action? halt;
    private bool began;

    /// <summary>Starts momentum. <paramref name="halt"/> stops the deceleration producing it when new input comes first.</summary>
    public void Start(Action halt)
    {
        Stop();
        this.halt = halt;
    }

    /// <summary>One step of the deceleration in the Mac's points, ignored once momentum stopped.</summary>
    public void Move(float dx, float dy)
    {
        if (halt is null || (dx == 0 && dy == 0))
        {
            return;
        }

        connection.Momentum(began ? MomentumPhase.Changed : MomentumPhase.Began, dx, dy);
        began = true;
    }

    /// <summary>The deceleration came to rest.</summary>
    public void End()
    {
        if (began)
        {
            connection.Momentum(MomentumPhase.Ended, 0, 0);
        }

        began = false;
        halt = null;
    }

    /// <summary>New input arrived, so momentum ends before it.</summary>
    public void Stop()
    {
        var stopping = halt;
        End();
        stopping?.Invoke();
    }
}
