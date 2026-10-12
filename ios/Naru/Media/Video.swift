import AVFoundation

/// 세션의 영상을 [layer]에 그립니다.
///
/// - 세션의 사건 스레드에서만 부르십시오. 디코딩과 표시는 시스템 하드웨어 디코더가 맡습니다.
/// - 설정(SPS, PPS)이 바뀌거나 표시가 실패하면 비우고, 키프레임이 올 때까지 넣지 않습니다.
///   요청이 서버에 닿지 않았을 수 있으므로, 기다리는 동안 프레임이 오면 일정 간격으로 다시 요청합니다.
/// - 비운 뒤 첫 프레임을 넣으면 [firstFrame]을 부릅니다. 그때까지 화면은 비어 있습니다.
final class Video: @unchecked Sendable {
    let layer = AVSampleBufferDisplayLayer()
    var requestKeyframe: () -> Void = {}
    private let firstFrame: () -> Void
    private var format: CMVideoFormatDescription?
    private var awaitingKeyframe = true
    private var requestedAt: ContinuousClock.Instant?
    private var shown = false

    init(firstFrame: @escaping () -> Void) {
        self.firstFrame = firstFrame
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

        let renderer = layer.sampleBufferRenderer

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
        layer.sampleBufferRenderer.flush()
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

    private static func format(_ config: DecoderConfig) -> CMVideoFormatDescription? {
        let sets = config.sps + config.pps
        let datas = sets.map { NSData(data: $0) }
        var pointers = datas.map { $0.bytes.assumingMemoryBound(to: UInt8.self) }
        var sizes = datas.map(\.length)
        var format: CMVideoFormatDescription?

        let status = CMVideoFormatDescriptionCreateFromH264ParameterSets(
            allocator: nil, parameterSetCount: sets.count, parameterSetPointers: &pointers,
            parameterSetSizes: &sizes, nalUnitHeaderLength: 4, formatDescriptionOut: &format
        )

        return status == noErr ? format : nil
    }

    // 시작 코드 없는 NAL을 4바이트 길이를 앞에 붙인 AVCC로 바꿉니다. 매개변수 집합은 형식 설명에 있으므로 뺍니다.
    private static func sample(_ unit: AccessUnit, format: CMVideoFormatDescription) -> CMSampleBuffer? {
        var data = Data()

        for nal in unit.nals where !nal.isEmpty && ![7, 8].contains(nal[nal.startIndex] & 0x1f) {
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
