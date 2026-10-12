import Foundation
import os

/// 세션 하나를 열고, 코어가 밀어 주는 사건을 영상, 소리, 화면 쪽으로 나눠 보냅니다.
///
/// - 사건은 세션의 사건 스레드에서 옵니다. 영상과 소리는 이 스레드에서 바로 넣고, 나머지는 [announce]로 넘깁니다.
/// - 세션이 끝난 뒤에 보낸 입력은 코어가 버립니다.
final class Connection: SessionListener, @unchecked Sendable {
    let video: Video
    private var audio: Audio?
    private var session: Session!
    private let announce: @Sendable (Event) -> Void
    private let log = Logger(subsystem: "dev.insd.naru", category: "Connection")

    // 세션은 연결하면서 사건 스레드를 시작하므로, 사건을 받을 영상이 먼저 있어야 합니다.
    @MainActor
    init(config: Config, announce: @escaping @Sendable (Event) -> Void, firstFrame: @escaping @Sendable () -> Void) {
        self.announce = announce
        video = Video(firstFrame: firstFrame)
        session = Session.connect(config: config, listener: self)
        video.requestKeyframe = { [weak self] in self?.session.requestKeyframe() }
    }

    func key(_ keysym: UInt32, down: Bool) { session.key(keysym: keysym, down: down) }

    func pointer(_ buttons: UInt8, x: UInt16, y: UInt16) { session.pointer(buttons: buttons, x: x, y: y) }

    func scroll(_ phase: Phase, dx: Float, dy: Float) { session.scroll(phase: phase, dx: dx, dy: dy) }

    func wheel(dx: Float, dy: Float) { session.wheel(dx: dx, dy: dy) }

    func magnify(_ phase: Phase, scale: Float) { session.magnify(phase: phase, scale: scale) }

    /// 보냈으면 `true`. 크기 한도를 넘으면 보내지 않고 `false`입니다.
    func clipboard(_ text: String) -> Bool { session.clipboard(text: text) }

    func resize(_ mode: DisplayMode) { session.resize(mode: mode) }

    func requestKeyframe() { session.requestKeyframe() }

    func disconnect() { session.disconnect() }

    // 영상과 소리는 초당 수십 번 오므로 화면 쪽에 넘기지 않고 로그에도 남기지 않습니다.
    func onEvent(event: Event) {
        switch event {
        case .frame, .audio, .stats:
            break
        default:
            log.info("\(Self.describe(event), privacy: .public)")
        }

        switch event {
        case let .frame(unit):
            video.queue(unit)
        case let .audio(frame):
            audio?.play(frame)
        case .stats:
            break
        case let .decoder(config):
            video.configure(config)
            announce(event)
        case .resetDecoder:
            video.reset()
            announce(event)
        case let .audioConfig(config):
            // 소리를 낼 수 없으면 영상만으로 계속합니다.
            audio = Audio(config: config)
            announce(event)
        case .closed:
            audio = nil
            announce(event)
        default:
            announce(event)
        }
    }

    // 로그에는 사건의 종류와 크기만 남깁니다. 클립보드 내용이나 키 바이트는 남기지 않습니다.
    private static func describe(_ event: Event) -> String {
        switch event {
        case let .progress(stage): "단계: \(stage)"
        case let .serverKey(der): "서버 키 \(der.count)바이트"
        case let .authenticated(serverProof): "인증 성공 (서버 증명 \(serverProof))"
        case let .layout(logicalWidth, logicalHeight, pixelWidth, pixelHeight):
            "레이아웃: 논리 \(logicalWidth)×\(logicalHeight), 픽셀 \(pixelWidth)×\(pixelHeight)"
        case let .decoder(config): "디코더 설정: SPS \(config.sps.count)개, PPS \(config.pps.count)개"
        case let .cursor(width, height, _, _, _): "커서 \(width)×\(height)"
        case let .serverName(name): "Mac 이름: \(name)"
        case let .source(source): "입력 소스: \(source.id) (보안 입력 \(source.secure))"
        case let .clipboard(text): "클립보드 \(text.count)자"
        case .resetDecoder: "디코더 재생성 요청"
        case let .closed(_, detail): "세션 종료: \(detail ?? "정상")"
        case let .audioConfig(config): "오디오 설정 \(config.count)바이트"
        default: "\(event)"
        }
    }
}
