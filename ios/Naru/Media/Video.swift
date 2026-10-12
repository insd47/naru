import AVFoundation
import VideoToolbox

/// Draws the session's video into [layer].
///
/// - Create on the main thread, then call only from the session's event thread. The system hardware decoder handles decoding and display.
/// - When the decoder config (SPS, PPS) changes or display fails, it flushes and enqueues nothing until a keyframe arrives.
///   The request may not have reached the server, so frames arriving while waiting trigger a new request at a fixed interval.
/// - Calls [firstFrame] when the first frame after a flush is enqueued. The screen stays blank until then.
/// - The codec follows the decoder config. HEVC 4:4:4 is requested only when [decodesHevc444] is true.
final class Video: @unchecked Sendable {
    let layer = AVSampleBufferDisplayLayer()
    var requestKeyframe: () -> Void = {}
    // The layer belongs to the main thread, but its renderer accepts samples from any thread.
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

    /// The server asked for the decoder to be recreated.
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

    /// Whether this device's hardware decoder can decode the server's HEVC 4:4:4
    ///
    /// - `VTIsHardwareDecodeSupported` ignores the chroma format, so this tries to create a hardware-only session from 4:4:4 parameter sets the server sent.
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

    // Parameter sets live in the format description, so they are removed from samples. H.264 uses SPS (7) and PPS (8); HEVC uses VPS (32), SPS (33), and PPS (34).
    private static func isParameterSet(_ nal: Data, codec: VideoCodec) -> Bool {
        let header = nal[nal.startIndex]

        return switch codec {
        case .h264: [7, 8].contains(header & 0x1f)
        case .hevc: (32...34).contains((header >> 1) & 0x3f)
        }
    }

    // Converts a NAL unit without a start code to the 4-byte length-prefixed format.
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

        // Displays frames immediately as they arrive, without timing.
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true) as? [NSMutableDictionary] {
            attachments.first?[kCMSampleAttachmentKey_DisplayImmediately] = true
        }

        return sample
    }
}

// HEVC 4:4:4 8-bit parameter sets a Mac sent for a 2420×1668 virtual display, used by [Video.decodesHevc444].
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
