import SwiftUI

/// Mac 하나의 행. 누르면 접속합니다.
///
/// - Screen Sharing 앱처럼 마우스를 올릴 때 Connect 버튼을 내지 않습니다. iPad에서는 포인터가 지날 때마다 행의 내용이 바뀌어 흔들립니다.
/// - 세션 중인 Mac은 Disconnect 버튼을 늘 보이고, 접속 중이면 진행 표시를 곁들입니다.
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

/// Screen Sharing 앱처럼 "Today at 20:48", "Aug 29, 2026 at 20:03"으로 씁니다.
func formatDate(_ date: Date) -> String {
    let time = date.formatted(.dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
    let calendar = Calendar.current

    if calendar.isDateInToday(date) { return "Today at \(time)" }
    if calendar.isDateInYesterday(date) { return "Yesterday at \(time)" }
    return "\(date.formatted(.dateTime.month(.abbreviated).day().year())) at \(time)"
}
