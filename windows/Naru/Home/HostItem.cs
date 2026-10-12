using System.ComponentModel;
using System.Globalization;
using System.Runtime.CompilerServices;
using Microsoft.UI.Xaml;
using Naru.Store;

namespace Naru.Home;

/// <summary>Where a Mac's session stands, as shown in the list.</summary>
public enum Presence
{
    Idle,
    Connecting,
    InSession,
}

/// <summary>One row of the Saved or Network list.</summary>
public sealed class HostItem : INotifyPropertyChanged
{
    private string title = "";
    private string subtitle = "";
    private string detail = "";
    private Presence presence;

    public event PropertyChangedEventHandler? PropertyChanged;

    public string Id { get; init; } = "";

    /// <summary>The saved Mac, or <c>null</c> for one found on the network.</summary>
    public Host? Host { get; set; }

    /// <summary>The Mac found on the network, or <c>null</c> for a saved one.</summary>
    public Found? Found { get; set; }

    public string Title { get => title; set => Set(ref title, value); }
    public string Subtitle { get => subtitle; set => Set(ref subtitle, value, nameof(SubtitleVisibility)); }
    public string Detail { get => detail; set => Set(ref detail, value); }
    public Presence Presence { get => presence; set => Set(ref presence, value, nameof(InSession), nameof(Connecting), nameof(Idle)); }

    public Visibility SubtitleVisibility => subtitle.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    public bool InSession => presence == Presence.InSession;
    public bool Connecting => presence == Presence.Connecting;
    public bool Idle => presence == Presence.Idle;

    // The list container names each row by this for screen readers.
    public override string ToString() => Title;

    public static HostItem From(Host host, Presence presence) =>
        new() { Id = host.Id, Host = host, Title = host.Title, Subtitle = host.Address, Detail = FormatDate(host.LastConnected), Presence = presence };

    public static HostItem From(Found found, Presence presence) =>
        new() { Id = found.Id, Found = found, Title = found.Name, Presence = presence };

    public void Update(Host host, Presence presence)
    {
        Host = host;
        Title = host.Title;
        Subtitle = host.Address;
        Detail = FormatDate(host.LastConnected);
        Presence = presence;
    }

    /// <summary>"Today at 21:29", "Yesterday at 21:29", or "Aug 29, 2026 at 20:03", as in Screen Sharing.</summary>
    public static string FormatDate(DateTimeOffset? time)
    {
        if (time is not { } value)
        {
            return "";
        }

        var local = value.ToLocalTime();
        var clock = local.ToString("HH:mm", CultureInfo.InvariantCulture);
        var today = DateTime.Today;

        if (local.Date == today)
        {
            return $"Today at {clock}";
        }

        if (local.Date == today.AddDays(-1))
        {
            return $"Yesterday at {clock}";
        }

        return $"{local.ToString("MMM d, yyyy", CultureInfo.InvariantCulture)} at {clock}";
    }

    private void Set<T>(ref T field, T value, params string[] dependents)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return;
        }

        field = value;
        Raise(dependents);
    }

    private void Raise(string[] dependents, [CallerMemberName] string? property = null)
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(property));

        foreach (var dependent in dependents)
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(dependent));
        }
    }
}
