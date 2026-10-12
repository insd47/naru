import SwiftUI

/// A row for one Mac. Tapping it connects.
///
/// - Unlike Screen Sharing, no Connect button appears on hover. On iPad the row content would shift every time the pointer passes over it.
/// - A Mac in a session always shows a Disconnect button, with a progress indicator while connecting.
struct HostRow: View {
    let title: String
    let subtitle: String?
    let detail: String?
    var active = false
    var connecting = false
    let onConnect: () -> Void
    var onDisconnect: () -> Void = {}
    var onInfo: (() -> Void)?

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "macmini")
                .font(.title)
                .foregroundStyle(.secondary)
                .frame(width: 40)

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let subtitle {
                    Text(subtitle).font(.subheadline).foregroundStyle(.secondary)
                }
            }

            Spacer()

            if active {
                if connecting { ProgressView() }
                Button("Disconnect", action: onDisconnect).buttonStyle(.bordered)
            } else if let detail {
                Text(detail).font(.subheadline).foregroundStyle(.secondary)
            }

            if let onInfo {
                Button("Edit", systemImage: "info.circle", action: onInfo)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
            }
        }
        .padding(.vertical, 6)
        .contentShape(.rect)
        .onTapGesture(perform: onConnect)
    }
}

/// Formats like Screen Sharing: "Today at 20:48", "Aug 29, 2026 at 20:03".
func formatDate(_ date: Date) -> String {
    let time = date.formatted(.dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
    let calendar = Calendar.current

    if calendar.isDateInToday(date) { return "Today at \(time)" }
    if calendar.isDateInYesterday(date) { return "Yesterday at \(time)" }
    return "\(date.formatted(.dateTime.month(.abbreviated).day().year())) at \(time)"
}
