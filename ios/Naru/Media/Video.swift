import AVFoundation
import VideoToolbox
import os

/// Decodes the session's video and draws it into [layer].
///
/// - Create on the main thread, then call only from the session's event thread, except [place], which belongs to the main thread.
/// - A decompression session decodes, and each band draws in a display layer of its own, stacked from the top. The
///   config says how many bands there are. HEVC comes in several, each a picture of its own, and the Mac sends only bands
///   that changed, so a band keeps its last picture until a new one replaces it. Pictures of several bands wait for
///   [present], so the bands of one instant show together like on Apple's client. H.264 is a single band covering the
///   screen, shown as it decodes.
/// - When the decoder config changes, the server asks for a new decoder, or decoding fails, a new decompression session
///   starts and decodes nothing until a keyframe arrives. The request may not have reached the server, so frames arriving
///   while waiting trigger a new request at a fixed interval.
/// - Calls [firstFrame] when the first picture after that is drawn. The screen stays blank until then.
/// - The codec follows the decoder config. HEVC 4:4:4 is requested only when [decodesHevc444] is true.
final class Video: @unchecked Sendable {
    let layer = CALayer()
    var requestKeyframe: () -> Void = {}
    // The Mac's screen in its own pixels, scaled to fit. It crops the bottom band's padding.
    private let screen = CALayer()
    private let firstFrame: @Sendable () -> Void
    private var format: CMVideoFormatDescription?
    private var session: VTDecompressionSession?
    private var awaitingKeyframe = true
    private var requestedAt: ContinuousClock.Instant?
    private let output = OSAllocatedUnfairLock(uncheckedState: Output())
    // Main thread only: a display layer per band, where to draw, the Mac's screen size in pixels, and the size of one band's picture
    private var bands: [AVSampleBufferDisplayLayer] = []
    private var bounds = CGRect.zero
    private var screenSize: CGSize?
    private var bandSize: CGSize?

    // Shared with the decoder's output handler, which may run on a thread of its own. The layers belong to the main
    // thread, but their renderers accept samples from any thread. The samples are ready and never change, so the lock
    // hands them between threads unchecked.
    private struct Output {
        var failed = false
        var shown = false
        // Renderers of the band layers, empty until the main thread has made them for the current band count
        var renderers: [AVSampleBufferVideoRenderer] = []
        // With several bands, the newest picture of each band waiting for [present]
        var held: [CMSampleBuffer?] = []
        // Pictures due on screen before their layer existed
        var unplaced: [CMSampleBuffer?] = []
    }

    @MainActor
    init(firstFrame: @escaping @Sendable () -> Void) {
        self.firstFrame = firstFrame
        screen.masksToBounds = true
        layer.addSublayer(screen)
    }

    /// Fits the Mac's screen, `size` in its pixels, into `bounds`.
    @MainActor
    func place(in bounds: CGRect, screen size: CGSize?) {
        self.bounds = bounds
        screenSize = size
        arrange()
    }

    func configure(_ config: DecoderConfig) {
        format = Self.format(config)

        // A new band count needs new layers, which only the main thread makes. Pictures due before then wait in `unplaced`.
        let count = Int(config.bands)
        let recount = output.withLockUnchecked { output in
            guard output.held.count != count else {
                return false
            }

            output.renderers = []
            output.held = Array(repeating: nil, count: count)
            output.unplaced = Array(repeating: nil, count: count)
            return true
        }

        let size = format.map { CMVideoFormatDescriptionGetPresentationDimensions($0, usePixelAspectRatio: false, useCleanAperture: true) }
        DispatchQueue.main.async { [self] in
            if recount {
                layOut(count)
            }

            bandSize = size
            arrange()
        }

        restart()
    }

    /// The server asked for the decoder to be recreated.
    func reset() {
        restart()
    }

    /// Every band of one instant has been queued, so the bands decoded since the last call show together.
    func present() {
        let ready = output.withLockUnchecked { output in
            defer { output.held = Array(repeating: nil, count: output.held.count) }
            return output.held.enumerated().compactMap { band, picture in picture.map { (band, $0) } }
        }

        show(ready)
    }

    func queue(_ unit: AccessUnit) {
        guard let format else {
            return
        }

        let band = Int(unit.band)

        if output.withLockUnchecked({ $0.failed }) {
            restart()
        }

        let renderers = output.withLockUnchecked { $0.renderers }

        if band < renderers.count, renderers[band].requiresFlushToResumeDecoding {
            renderers[band].flush()
        }

        if awaitingKeyframe {
            guard unit.keyframe else {
                requestIfDue()
                return
            }

            awaitingKeyframe = false
        }

        guard let session, band < output.withLockUnchecked({ $0.held.count }), let sample = Self.sample(unit, format: format) else {
            return
        }

        VTDecompressionSessionDecodeFrame(session, sampleBuffer: sample, flags: [], infoFlagsOut: nil) { [weak self] status, _, image, _, _ in
            if status != noErr {
                log.error("cannot decode a frame: \(status)")
            }

            self?.draw(status == noErr ? image : nil, band: band)
        }
    }

    private func draw(_ image: CVImageBuffer?, band: Int) {
        guard let image else {
            output.withLockUnchecked { $0.failed = true }
            return
        }

        guard let picture = Self.picture(image) else {
            return
        }

        let ready = output.withLockUnchecked { output in
            guard output.held.count > 1 else {
                return [(band, picture)]
            }

            output.held[band] = picture
            return []
        }

        show(ready)
    }

    private func show(_ pictures: [(band: Int, picture: CMSampleBuffer)]) {
        guard !pictures.isEmpty else {
            return
        }

        let renderers = output.withLockUnchecked { output in
            if output.renderers.isEmpty {
                for (band, picture) in pictures where band < output.unplaced.count {
                    output.unplaced[band] = picture
                }
            }

            return output.renderers
        }

        for (band, picture) in pictures where band < renderers.count {
            renderers[band].enqueue(picture)
        }

        let first = output.withLockUnchecked { output in
            defer { output.shown = true }
            return !output.shown
        }

        if first {
            let (band, picture) = pictures[0]
            let size = picture.imageBuffer.map { CGSize(width: CVPixelBufferGetWidth($0), height: CVPixelBufferGetHeight($0)) } ?? .zero
            let renderer = renderers[band]
            log.log("first picture \(Int(size.width))×\(Int(size.height)) in band \(band), display \(renderer.status.rawValue) \(renderer.error?.localizedDescription ?? "", privacy: .public)")
            firstFrame()
        }
    }

    // A new session for a new generation of references. The bands keep their pictures, since after a reset the Mac may
    // resend only the top band.
    private func restart() {
        if let session {
            VTDecompressionSessionInvalidate(session)
        }

        session = format.flatMap(Self.session)
        awaitingKeyframe = true
        output.withLockUnchecked { output in
            output = Output(renderers: output.renderers, held: Array(repeating: nil, count: output.held.count), unplaced: output.unplaced)
        }
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

    // Replaces the band layers and draws the pictures that were due before they existed.
    @MainActor
    private func layOut(_ count: Int) {
        bands.forEach { $0.removeFromSuperlayer() }
        bands = (0..<count).map { _ in AVSampleBufferDisplayLayer() }

        for band in bands {
            band.videoGravity = .resize
            screen.addSublayer(band)
        }

        let renderers = bands.map(\.sampleBufferRenderer)
        let unplaced = output.withLockUnchecked { output in
            defer { output.unplaced = Array(repeating: nil, count: output.unplaced.count) }
            output.renderers = renderers
            return output.unplaced
        }

        for (renderer, picture) in zip(renderers, unplaced) {
            if let picture { renderer.enqueue(picture) }
        }
    }

    @MainActor
    private func arrange() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }

        layer.frame = bounds

        guard let bandSize else {
            return
        }

        let size = screenSize ?? bandSize

        guard size.width > 0, size.height > 0 else {
            return
        }

        let scale = min(bounds.width / size.width, bounds.height / size.height)
        screen.bounds = CGRect(origin: .zero, size: size)
        screen.position = CGPoint(x: bounds.midX, y: bounds.midY)
        screen.setAffineTransform(CGAffineTransform(scaleX: scale, y: scale))

        for (index, band) in bands.enumerated() {
            band.frame = CGRect(origin: CGPoint(x: 0, y: CGFloat(index) * bandSize.height), size: bandSize)
        }
    }

    /// Whether this device's hardware decoder can decode the server's HEVC 4:4:4
    ///
    /// - `VTIsHardwareDecodeSupported` ignores the chroma format, so this tries to create a hardware-only session from 4:4:4 parameter sets the server sent.
    static let decodesHevc444: Bool = {
        guard let format = format(DecoderConfig(codec: .hevc, bands: 1, profile: 4, compatibility: 0, level: 150, vps: [hevcVps], sps: [hevcSps], pps: [hevcPps])) else {
            return false
        }

        let specification = [kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder: true] as CFDictionary
        var session: VTDecompressionSession?
        // A display layer shows only pictures backed by an IOSurface, which a software decoder does not use by itself.
        let attributes = [kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary] as CFDictionary
        let status = VTDecompressionSessionCreate(
            allocator: nil, formatDescription: format, decoderSpecification: specification,
            imageBufferAttributes: attributes, outputCallback: nil, decompressionSessionOut: &session
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

        return sample
    }

    // Hardware is preferred but not required: HEVC 4:4:4 is only requested where hardware decodes it, and H.264 may fall
    // back to software, as in the simulator.
    private static func session(_ format: CMVideoFormatDescription) -> VTDecompressionSession? {
        let specification = [kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: true] as CFDictionary
        var session: VTDecompressionSession?
        // A display layer shows only pictures backed by an IOSurface, which a software decoder does not use by itself.
        let attributes = [kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary] as CFDictionary
        let status = VTDecompressionSessionCreate(
            allocator: nil, formatDescription: format, decoderSpecification: specification,
            imageBufferAttributes: attributes, outputCallback: nil, decompressionSessionOut: &session
        )

        guard let session else {
            log.error("cannot create the decoder: \(status)")
            return nil
        }

        VTSessionSetProperty(session, key: kVTDecompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        return session
    }

    // Wraps a decoded picture for a display layer, to be shown as soon as it arrives.
    private static func picture(_ image: CVImageBuffer) -> CMSampleBuffer? {
        var format: CMVideoFormatDescription?
        var sample: CMSampleBuffer?
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: CMClockGetTime(CMClockGetHostTimeClock()), decodeTimeStamp: .invalid)

        guard CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: image, formatDescriptionOut: &format) == noErr,
              let format,
              CMSampleBufferCreateReadyWithImageBuffer(
                  allocator: nil, imageBuffer: image, formatDescription: format, sampleTiming: &timing, sampleBufferOut: &sample
              ) == noErr,
              let sample
        else {
            return nil
        }

        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true) as? [NSMutableDictionary] {
            attachments.first?[kCMSampleAttachmentKey_DisplayImmediately] = true
        }

        return sample
    }
}

private let log = Logger(subsystem: Diagnostics.subsystem, category: "Video")

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
