import AVFoundation
import VideoToolbox

/// 세션의 영상을 [layer]에 그립니다.
///
/// - 메인 스레드에서 만들고, 그 뒤로는 세션의 사건 스레드에서만 부르십시오. 디코딩과 표시는 시스템 하드웨어 디코더가 맡습니다.
/// - 설정(SPS, PPS)이 바뀌거나 표시가 실패하면 비우고, 키프레임이 올 때까지 넣지 않습니다.
///   요청이 서버에 닿지 않았을 수 있으므로, 기다리는 동안 프레임이 오면 일정 간격으로 다시 요청합니다.
/// - 비운 뒤 첫 프레임을 넣으면 [firstFrame]을 부릅니다. 그때까지 화면은 비어 있습니다.
/// - 코덱은 설정을 따릅니다. HEVC 4:4:4는 [decodesHevc444]일 때만 요청합니다.
final class Video: @unchecked Sendable {
    let layer = AVSampleBufferDisplayLayer()
    var requestKeyframe: () -> Void = {}
    // 레이어는 메인 스레드 것이지만, 렌더러는 어느 스레드에서나 넣을 수 있습니다.
    private let renderer: AVSampleBufferVideoRenderer
    private let firstFrame: () -> Void
    private var format: CMVideoFormatDescription?
    private var awaitingKeyframe = true
    private var requestedAt: ContinuousClock.Instant?
    private var shown = false

    @MainActor
    init(firstFrame: @escaping () -> Void) {
        self.firstFrame = firstFrame
        renderer = layer.sampleBufferRenderer
        layer.videoGravity = .resizeAspect
    }

    func configure(_ config: DecoderConfig) {
        format = Self.format(config)
        flush()
    }

    /// 서버가 디코더를 새로 만들라고 했습니다.
    func reset() {
        flush()
    }

    func queue(_ unit: AccessUnit) {
        guard let format else {
            return
        }

        if renderer.status == .failed || renderer.requiresFlushToResumeDecoding {
            flush()
        }

        if awaitingKeyframe {
            guard unit.keyframe else {
                requestIfDue()
                return
            }

            awaitingKeyframe = false
        }

        guard let sample = Self.sample(unit, format: format) else {
            return
        }

        renderer.enqueue(sample)

        if !shown {
            shown = true
            firstFrame()
        }
    }

    private func flush() {
        renderer.flush()
        awaitingKeyframe = true
        shown = false
        requestedAt = nil
        requestIfDue()
    }

    private func requestIfDue() {
        let now = ContinuousClock.now

        if requestedAt.map({ now - $0 >= .seconds(1) }) ?? true {
            requestedAt = now
            requestKeyframe()
        }
    }

    /// 이 기기의 하드웨어 디코더가 서버의 HEVC 4:4:4를 풀 수 있는지 여부
    ///
    /// - `VTIsHardwareDecodeSupported`는 크로마 형식을 가리지 않으므로, 서버가 보낸 4:4:4 파라미터 세트로 하드웨어 전용 세션을 만들어 봅니다.
    static let decodesHevc444: Bool = {
        guard let format = format(DecoderConfig(codec: .hevc, profile: 4, compatibility: 0, level: 150, vps: [hevcVps], sps: [hevcSps], pps: [hevcPps])) else {
            return false
        }

        let specification = [kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder: true] as CFDictionary
        var session: VTDecompressionSession?
        let status = VTDecompressionSessionCreate(
            allocator: nil, formatDescription: format, decoderSpecification: specification,
            imageBufferAttributes: nil, outputCallback: nil, decompressionSessionOut: &session
        )

        if let session { VTDecompressionSessionInvalidate(session) }
        return status == noErr
    }()

    private static func format(_ config: DecoderConfig) -> CMVideoFormatDescription? {
        let sets = config.vps + config.sps + config.pps
        let datas = sets.map { NSData(data: $0) }
        var pointers = datas.map { $0.bytes.assumingMemoryBound(to: UInt8.self) }
        var sizes = datas.map(\.length)
        var format: CMVideoFormatDescription?

        let status = switch config.codec {
        case .h264:
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                allocator: nil, parameterSetCount: sets.count, parameterSetPointers: &pointers,
                parameterSetSizes: &sizes, nalUnitHeaderLength: 4, formatDescriptionOut: &format
            )
        case .hevc:
            CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                allocator: nil, parameterSetCount: sets.count, parameterSetPointers: &pointers,
                parameterSetSizes: &sizes, nalUnitHeaderLength: 4, extensions: nil, formatDescriptionOut: &format
            )
        }

        return status == noErr ? format : nil
    }

    // 파라미터 세트는 형식 설명에 있으므로 샘플에서 뺍니다. H.264는 SPS(7)와 PPS(8), HEVC는 VPS(32), SPS(33), PPS(34)입니다.
    private static func isParameterSet(_ nal: Data, codec: VideoCodec) -> Bool {
        let header = nal[nal.startIndex]

        return switch codec {
        case .h264: [7, 8].contains(header & 0x1f)
        case .hevc: (32...34).contains((header >> 1) & 0x3f)
        }
    }

    // 시작 코드 없는 NAL을 4바이트 길이를 앞에 붙인 형식으로 바꿉니다.
    private static func sample(_ unit: AccessUnit, format: CMVideoFormatDescription) -> CMSampleBuffer? {
        var data = Data()

        for nal in unit.nals where !nal.isEmpty && !isParameterSet(nal, codec: unit.codec) {
            withUnsafeBytes(of: UInt32(nal.count).bigEndian) { data.append(contentsOf: $0) }
            data.append(nal)
        }

        var block: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(
            allocator: nil, memoryBlock: nil, blockLength: data.count, blockAllocator: nil,
            customBlockSource: nil, offsetToData: 0, dataLength: data.count, flags: 0, blockBufferOut: &block
        ) == noErr, let block else {
            return nil
        }

        _ = data.withUnsafeBytes { CMBlockBufferReplaceDataBytes(with: $0.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: data.count) }

        var sample: CMSampleBuffer?
        var size = data.count
        guard CMSampleBufferCreateReady(
            allocator: nil, dataBuffer: block, formatDescription: format, sampleCount: 1, sampleTimingEntryCount: 0,
            sampleTimingArray: nil, sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample
        ) == noErr, let sample else {
            return nil
        }

        // 시각을 맞추지 않고 받는 대로 바로 그립니다.
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true) as? [NSMutableDictionary] {
            attachments.first?[kCMSampleAttachmentKey_DisplayImmediately] = true
        }

        return sample
    }
}

// macOS 27 Mac mini가 2420×1668 가상 디스플레이로 보낸 HEVC 4:4:4 8비트 파라미터 세트입니다(2026-10-12 캡처). [Video.decodesHevc444]에 씁니다.
private let hevcVps = Data([
    0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00,
    0x00, 0x96, 0x0c, 0xc0, 0x90,
])
private let hevcSps = Data([
    0x42, 0x01, 0x01, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00, 0x00, 0x96, 0x90,
    0x00, 0x26, 0x04, 0x00, 0xd2, 0x38, 0xd8, 0xdc, 0x40, 0x67, 0x72, 0x29, 0x0c, 0x5c, 0xfe, 0x27, 0xe1, 0x7f,
    0x43, 0x7e, 0xa1, 0xff, 0x54, 0x11, 0xfa, 0xa8, 0x27, 0xf5, 0x54, 0x15, 0xfa, 0xaa, 0x82, 0xff, 0x55, 0x54,
    0x19, 0xfa, 0xaa, 0xa9, 0x4d, 0xc3, 0x03, 0x40, 0x40, 0x20,
])
private let hevcPps = Data([0x44, 0x01, 0xc0, 0x72, 0xf0, 0x5b, 0x24])
