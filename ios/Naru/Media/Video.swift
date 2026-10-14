import AVFoundation
import Metal
import VideoToolbox
import os

/// Decodes the session's video and draws it into [layer].
///
/// - Create on the main thread. [place] belongs to the main thread; the other calls come from the session's event thread
///   and only hand work to a serial queue of the video's own, so a slow decode never holds up the session's events.
/// - When a frame has waited on the queue for half a second, the waiting frames are dropped and it resumes from a keyframe.
/// - A decompression session decodes to BGRA Metal textures. The config says how many bands there are: HEVC comes in
///   several, each a picture of its own stacked from the top, and the Mac sends only bands that changed. H.264 is a
///   single band covering the screen.
/// - Bands are copied into a canvas that keeps each band's last picture. Each update of the canvas becomes one whole
///   picture of the screen, and one display layer shows it, so the system still decides when it reaches the screen.
///   Pictures of several bands wait for [present], so the bands of one instant show together like on Apple's client; a
///   display layer per band would pick its own refresh. A single band shows as it decodes.
/// - When the decoder config changes, the server asks for a new decoder, or decoding fails, a new decompression session
///   starts and decodes nothing until a keyframe arrives. The request may not have reached the server, so frames arriving
///   while waiting trigger a new request at a fixed interval.
/// - Calls [firstFrame] when the first picture after that is drawn. The screen stays blank until then.
/// - The codec follows the decoder config. HEVC 4:4:4 is requested only when [decodesHevc444] is true.
final class Video: @unchecked Sendable {
    let layer = CALayer()
    var requestKeyframe: () -> Void = {}
    private let display = AVSampleBufferDisplayLayer()
    // The display layer belongs to the main thread, but its renderer accepts pictures from any thread.
    private let renderer: AVSampleBufferVideoRenderer
    private let device: MTLDevice
    private let commands: MTLCommandQueue
    private let textures: CVMetalTextureCache
    private let firstFrame: @Sendable () -> Void
    private let work = DispatchQueue(label: "naru-video", qos: .userInteractive)
    private let output = OSAllocatedUnfairLock(uncheckedState: Output())
    // Queue only
    private var format: CMVideoFormatDescription?
    private var session: VTDecompressionSession?
    private var awaitingKeyframe = true
    private var requestedAt: ContinuousClock.Instant?
    private var canvas: MTLTexture?
    // Whole pictures of the screen, as large as the Mac's screen, which crops the bottom band's padding
    private var pictures: CVPixelBufferPool?
    private var visible: CGSize?
    private var pictureSize = (0, 0)
    private var shown = false
    // Counts logged with every twentieth stats event, about every 10 seconds
    private var counts = Counts()

    // Shared with the decoder's output handler, which may run on a thread of its own. Decoded pictures are not changed
    // once out, so the lock hands them between threads unchecked.
    private struct Output {
        var failed = false
        // The newest picture of each band since the last show
        var held: [CVMetalTexture?] = []
    }

    // How long a frame may wait on the queue before the waiting ones are dropped. Measured in time, since the count per
    // second depends on the frame rate and the number of bands. Decoding catches up far faster than real time, while a
    // drop costs a keyframe, so this tolerates passing hiccups such as a new decoder.
    private static let backlog = Duration.milliseconds(500)

    private struct Counts {
        var reports = 0
        var frames = 0
        var dropped = 0
        var presented = 0
        var decoding = Duration.zero
        var slowestDecode = Duration.zero
    }

    @MainActor
    init(firstFrame: @escaping @Sendable () -> Void) {
        self.firstFrame = firstFrame
        device = MTLCreateSystemDefaultDevice()!
        commands = device.makeCommandQueue()!

        var cache: CVMetalTextureCache?
        CVMetalTextureCacheCreate(nil, nil, device, nil, &cache)
        textures = cache!

        renderer = display.sampleBufferRenderer
        display.videoGravity = .resizeAspect
        layer.addSublayer(display)
    }

    /// Fits the Mac's screen, `size` in its pixels, into `bounds`.
    @MainActor
    func place(in bounds: CGRect, screen size: CGSize?) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.frame = bounds
        display.frame = CGRect(origin: .zero, size: bounds.size)
        CATransaction.commit()

        work.async { [self] in visible = size }
    }

    func configure(_ config: DecoderConfig) {
        work.async { [self] in setUp(config) }
    }

    /// The server asked for the decoder to be recreated.
    func reset() {
        work.async { [self] in restart() }
    }

    /// Every band of one instant has been queued, so the bands decoded since the last call show together.
    func present() {
        work.async { [self] in
            // The decoder may hand pictures over later on a thread of its own, so the instant's last bands are waited for.
            if let session {
                VTDecompressionSessionWaitForAsynchronousFrames(session)
            }

            counts.presented += 1
            show()
        }
    }

    func queue(_ unit: AccessUnit) {
        let queued = ContinuousClock.now
        work.async { [self] in decode(unit, queued: queued) }
    }

    func report(_ stats: MediaStats) {
        work.async { [self] in
            counts.reports += 1

            guard counts.reports >= 20 else {
                return
            }

            let c = counts
            log.log("""
                received (10 s): frames \(c.frames) (dropped \(c.dropped)), presented \(c.presented), decode \
                \(Self.milliseconds(c.decoding / max(c.frames, 1)), privacy: .public) avg \
                \(Self.milliseconds(c.slowestDecode), privacy: .public) max | missing \(stats.missing), lost \(stats.losses)
                """)
            counts = Counts()
        }
    }

    private func setUp(_ config: DecoderConfig) {
        format = Self.format(config)

        let count = Int(config.bands)
        output.withLockUnchecked { output in
            if output.held.count != count {
                output.held = Array(repeating: nil, count: count)
            }
        }

        // The canvas stacks the bands; a new size starts a new one.
        let size = format.map { CMVideoFormatDescriptionGetPresentationDimensions($0, usePixelAspectRatio: false, useCleanAperture: true) }
        let next = size.map { CGSize(width: $0.width, height: $0.height * CGFloat(count)) }

        if let next, canvas.map({ CGSize(width: $0.width, height: $0.height) }) != next {
            let description = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: .bgra8Unorm, width: Int(next.width), height: Int(next.height), mipmapped: false
            )
            description.usage = [.shaderRead, .renderTarget]
            description.storageMode = .private
            canvas = device.makeTexture(descriptor: description)
            clear()
        }

        restart()
    }

    private func decode(_ unit: AccessUnit, queued: ContinuousClock.Instant) {
        counts.frames += 1

        guard let format else {
            return
        }

        // Decoding fell behind the network: drop the backlog and resume from a keyframe.
        if ContinuousClock.now - queued >= Self.backlog, !unit.keyframe {
            if !awaitingKeyframe {
                log.log("video backed up; dropping pending frames")
                awaitingKeyframe = true
                requestedAt = nil
                requestIfDue()
            }

            counts.dropped += 1
            return
        }

        if output.withLockUnchecked({ $0.failed }) {
            restart()
        }

        if awaitingKeyframe {
            guard unit.keyframe else {
                requestIfDue()
                return
            }

            awaitingKeyframe = false
        }

        let band = Int(unit.band)

        guard let session, band < output.withLockUnchecked({ $0.held.count }), let sample = Self.sample(unit, format: format) else {
            return
        }

        let started = ContinuousClock.now
        VTDecompressionSessionDecodeFrame(session, sampleBuffer: sample, flags: [], infoFlagsOut: nil) { [weak self] status, _, image, _, _ in
            if status != noErr {
                log.error("cannot decode a frame: \(status)")
            }

            self?.hold(status == noErr ? image : nil, band: band)
        }
        let decoding = ContinuousClock.now - started
        counts.decoding += decoding
        counts.slowestDecode = max(counts.slowestDecode, decoding)

        if output.withLockUnchecked({ $0.held.count }) == 1 {
            show()
        }
    }

    // Runs in the decoder's output handler.
    private func hold(_ image: CVImageBuffer?, band: Int) {
        guard let image else {
            output.withLockUnchecked { $0.failed = true }
            return
        }

        var texture: CVMetalTexture?
        CVMetalTextureCacheCreateTextureFromImage(
            nil, textures, image, nil, .bgra8Unorm, CVPixelBufferGetWidth(image), CVPixelBufferGetHeight(image), 0, &texture
        )

        output.withLockUnchecked { output in
            if band < output.held.count {
                output.held[band] = texture
            }
        }
    }

    // A new canvas holds whatever the memory had, so it starts black until the bands arrive.
    private func clear() {
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = canvas
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 1)
        pass.colorAttachments[0].storeAction = .store

        guard let buffer = commands.makeCommandBuffer(), let encoder = buffer.makeRenderCommandEncoder(descriptor: pass) else {
            return
        }

        encoder.endEncoding()
        buffer.commit()
    }

    // Copies the bands held since the last show into the canvas and sends the screen they make up to the display layer.
    private func show() {
        let held = output.withLockUnchecked { output in
            defer { output.held = Array(repeating: nil, count: output.held.count) }
            return output.held
        }

        guard let canvas, held.contains(where: { $0 != nil }), let buffer = commands.makeCommandBuffer(),
              let blit = buffer.makeBlitCommandEncoder() else {
            return
        }

        for case let (band, picture?) in held.enumerated() {
            guard let texture = CVMetalTextureGetTexture(picture) else {
                continue
            }

            let top = band * texture.height
            let height = min(texture.height, canvas.height - top)

            if height > 0 {
                let size = MTLSize(width: min(texture.width, canvas.width), height: height, depth: 1)
                blit.copy(
                    from: texture, sourceSlice: 0, sourceLevel: 0, sourceOrigin: MTLOrigin(), sourceSize: size,
                    to: canvas, destinationSlice: 0, destinationLevel: 0, destinationOrigin: MTLOrigin(x: 0, y: top, z: 0)
                )
            }
        }

        guard let picture = whole(canvas, encoding: blit) else {
            blit.endEncoding()
            buffer.commit()
            return
        }

        blit.endEncoding()

        // The decoder's pictures stay alive until the copies are done, and the whole picture goes out once it is drawn.
        nonisolated(unsafe) let used = (held, picture)
        buffer.addCompletedHandler { [self] _ in
            withExtendedLifetime(used.0) {}

            if renderer.requiresFlushToResumeDecoding {
                renderer.flush()
            }

            if let sample = Self.sample(showing: used.1.image) {
                renderer.enqueue(sample)
            }
        }
        buffer.commit()

        if !shown {
            shown = true
            firstFrame()
        }
    }

    // A new session for a new generation of references. The canvas keeps the bands, since after a reset the Mac may
    // resend only the top band.
    private func restart() {
        if let session {
            VTDecompressionSessionInvalidate(session)
        }

        session = format.flatMap(Self.session)
        awaitingKeyframe = true
        shown = false
        output.withLockUnchecked { output in
            output = Output(held: Array(repeating: nil, count: output.held.count))
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

    // Copies the visible part of the canvas into a whole picture of the screen from the pool.
    private func whole(_ canvas: MTLTexture, encoding blit: MTLBlitCommandEncoder) -> (image: CVPixelBuffer, texture: CVMetalTexture)? {
        let width = canvas.width
        let height = min(canvas.height, visible.map { Int($0.height) } ?? canvas.height)

        if pictures == nil || pictureSize != (width, height) {
            let attributes = [
                kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
                kCVPixelBufferWidthKey: width,
                kCVPixelBufferHeightKey: height,
                kCVPixelBufferMetalCompatibilityKey: true,
                kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
            ] as CFDictionary
            CVPixelBufferPoolCreate(nil, nil, attributes, &pictures)
            pictureSize = (width, height)
        }

        var image: CVPixelBuffer?
        var wrapped: CVMetalTexture?

        guard let pictures, CVPixelBufferPoolCreatePixelBuffer(nil, pictures, &image) == kCVReturnSuccess, let image,
              CVMetalTextureCacheCreateTextureFromImage(nil, textures, image, nil, .bgra8Unorm, width, height, 0, &wrapped) == kCVReturnSuccess,
              let wrapped, let texture = CVMetalTextureGetTexture(wrapped) else {
            return nil
        }

        blit.copy(
            from: canvas, sourceSlice: 0, sourceLevel: 0, sourceOrigin: MTLOrigin(), sourceSize: MTLSize(width: width, height: height, depth: 1),
            to: texture, destinationSlice: 0, destinationLevel: 0, destinationOrigin: MTLOrigin()
        )
        return (image, wrapped)
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
        // Metal reads the pictures as BGRA textures, which a software decoder does not produce by itself.
        let attributes = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
            kCVPixelBufferMetalCompatibilityKey: true,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
        ] as CFDictionary
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
        // Metal reads the pictures as BGRA textures, which a software decoder does not produce by itself.
        let attributes = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
            kCVPixelBufferMetalCompatibilityKey: true,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
        ] as CFDictionary
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

    // Wraps a whole picture for the display layer, shown as soon as it arrives.
    private static func sample(showing image: CVPixelBuffer) -> CMSampleBuffer? {
        var format: CMVideoFormatDescription?
        var sample: CMSampleBuffer?
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: .zero, decodeTimeStamp: .invalid)

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

    private static func milliseconds(_ duration: Duration) -> String {
        duration.formatted(.units(allowed: [.milliseconds], fractionalPart: .show(length: 1)))
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
