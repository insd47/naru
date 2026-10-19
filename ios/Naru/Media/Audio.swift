import AudioToolbox
import AVFoundation
import Synchronization
import os

/// Plays the Mac's audio: decodes AAC-ELD to PCM with an AudioConverter and plays it from a buffer the audio device
/// pulls from, as Fraunhofer's AAC-ELD notes for iOS and Moonlight's player do.
///
/// - Call only from the session's event thread. Frames come paced by the session, which keeps them with the picture.
/// - The device takes sound when it needs it. When the buffer runs dry, it plays silence until enough sound waits
///   again, so a late frame costs a gap rather than the stream. An AudioQueue fed compressed frames instead kept
///   decoding them while playing silence for good once it fell behind.
/// - At most `maxBacklog` of sound waits, and frames beyond it are dropped after decoding, so the decoder keeps its
///   state. A burst of sound, as when the session catches up after the network paused, skips ahead with the picture.
/// - The system stops the engine on route changes and interruptions, so a stopped engine is started again.
/// - The AudioSpecificConfig is wrapped in an MPEG-4 ES descriptor, the magic cookie format AudioToolbox reads, so the
///   system determines the sample rate and channels.
final class Audio {
    // Starting the engine is tried no more often than this.
    private static let restartInterval = Duration.milliseconds(500)

    private let converter: AudioConverterRef
    private let channels: Int
    private let rate: Double
    // Decoded PCM of one frame, interleaved
    private let pcm: UnsafeMutableBufferPointer<Float>
    private let ring: Ring
    private let engine = AVAudioEngine()
    private let maxBacklog: Int
    private var restarted = ContinuousClock.now
    // Diagnostics: the last frame's timestamp, frames dropped in a row for a backlog, frames that failed to decode, and
    // when the output was last reported
    private var lastTimestamp: UInt32?
    private var dropped = 0
    private var failed = 0
    private var reported = ContinuousClock.now

    init?(config: Data) {
        let cookie = Self.cookie(config)
        var input = AudioStreamBasicDescription()
        input.mFormatID = kAudioFormatMPEG4AAC_ELD_SBR
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)

        let described = cookie.withUnsafeBytes {
            AudioFormatGetProperty(kAudioFormatProperty_FormatInfo, UInt32(cookie.count), $0.baseAddress, &size, &input)
        }

        guard described == noErr, let format = AVAudioFormat(standardFormatWithSampleRate: input.mSampleRate, channels: input.mChannelsPerFrame) else {
            return nil
        }

        let channels = Int(input.mChannelsPerFrame)
        var output = AudioStreamBasicDescription(
            mSampleRate: input.mSampleRate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
            mBytesPerPacket: UInt32(4 * channels),
            mFramesPerPacket: 1,
            mBytesPerFrame: UInt32(4 * channels),
            mChannelsPerFrame: UInt32(channels),
            mBitsPerChannel: 32,
            mReserved: 0
        )
        var converter: AudioConverterRef?

        guard AudioConverterNew(&input, &output, &converter) == noErr, let converter else {
            return nil
        }

        let cookieSet = cookie.withUnsafeBytes {
            AudioConverterSetProperty(converter, kAudioConverterDecompressionMagicCookie, UInt32(cookie.count), $0.baseAddress!)
        }

        guard cookieSet == noErr else {
            AudioConverterDispose(converter)
            return nil
        }

        // Playing starts once 40 ms waits, and at most 100 ms waits.
        let rate = input.mSampleRate
        let ring = Ring(channels: channels, capacity: Int(rate), prime: Int(rate * 0.04))
        self.converter = converter
        self.channels = channels
        self.rate = rate
        self.pcm = .allocate(capacity: Int(input.mFramesPerPacket) * channels)
        self.ring = ring
        self.maxBacklog = Int(rate * 0.1)

        let node = AVAudioSourceNode(format: format) { silence, _, frames, list in
            silence.pointee = ObjCBool(ring.render(UnsafeMutableAudioBufferListPointer(list), frames: Int(frames)))
            return noErr
        }
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: format)

        try? AVAudioSession.sharedInstance().setCategory(.playback, options: .mixWithOthers)
        try? AVAudioSession.sharedInstance().setActive(true)
        start()
    }

    // ES descriptor > decoder config descriptor (MPEG-4 audio, audio stream) > decoder specific info, then the SL config descriptor.
    // AudioToolbox rejects a bare AudioSpecificConfig with kAudioFormatUnsupportedDataFormatError.
    private static func cookie(_ config: Data) -> Data {
        func descriptor(_ tag: UInt8, _ body: Data) -> Data { Data([tag, UInt8(body.count)]) + body }

        let decoderConfig = descriptor(0x04, Data([0x40, 0x15] + [UInt8](repeating: 0, count: 11)) + descriptor(0x05, config))
        return descriptor(0x03, Data([0, 0, 0]) + decoderConfig + descriptor(0x06, Data([0x02])))
    }

    deinit {
        engine.stop()
        AudioConverterDispose(converter)
        pcm.deallocate()
    }

    func play(_ frame: AudioFrame) {
        let now = ContinuousClock.now

        // Each frame is 480 samples at 48 kHz, so any other step means frames the network lost.
        if let lastTimestamp, frame.timestamp &- lastTimestamp != 480 {
            log.log("audio stream skipped from \(lastTimestamp) to \(frame.timestamp)")
        }
        lastTimestamp = frame.timestamp

        if !engine.isRunning, now - restarted >= Self.restartInterval {
            restarted = now
            start()
        }

        if now - reported >= .seconds(10) {
            reported = now
            report()
        }

        guard let frames = decode(frame.data) else {
            failed += 1
            return
        }

        if ring.buffered >= maxBacklog {
            dropped += 1
            return
        }

        if dropped > 0 {
            log.log("audio dropped \(self.dropped) frames to keep up")
            dropped = 0
        }

        ring.write(pcm.baseAddress!, frames: frames)
    }

    private func start() {
        do {
            try engine.start()
        } catch {
            log.error("audio engine could not start: \(error, privacy: .public)")
        }
    }

    // Decodes one access unit into `pcm` and returns the frames decoded.
    private func decode(_ data: Data) -> Int? {
        data.withUnsafeBytes { bytes in
            let packet = AudioStreamPacketDescription(mStartOffset: 0, mVariableFramesInPacket: 0, mDataByteSize: UInt32(data.count))
            var input = Input(bytes: bytes.baseAddress, packet: packet)
            var frames = UInt32(pcm.count / channels)
            var list = AudioBufferList(
                mNumberBuffers: 1,
                mBuffers: AudioBuffer(mNumberChannels: UInt32(channels), mDataByteSize: UInt32(pcm.count * 4), mData: pcm.baseAddress)
            )

            let status = withUnsafeMutablePointer(to: &input) { AudioConverterFillComplexBuffer(converter, supply, $0, &frames, &list, nil) }
            return status == noErr || status == drained ? Int(frames) : nil
        }
    }

    private func report() {
        let session = AVAudioSession.sharedInstance()
        let route = session.currentRoute.outputs.map(\.portType.rawValue).joined(separator: ", ")
        let underruns = ring.underruns.exchange(0, ordering: .relaxed)
        log.log("""
            audio (10 s): waiting \(Int(Double(self.ring.buffered) / self.rate * 1000)) ms, underruns \(underruns), \
            failed \(self.failed), running \(self.engine.isRunning), route \(route, privacy: .public), \
            io \(Int(session.ioBufferDuration * 1000)) ms
            """)
        failed = 0
    }
}

/// Decoded sound between the session's event thread, which writes it, and the device's render thread, which reads it.
///
/// - One writer and one reader, each moving only its own index, so neither waits for the other.
/// - When the reader runs dry, it plays silence until `prime` frames wait again.
private final class Ring: @unchecked Sendable {
    let underruns = Atomic<Int>(0)

    private let channels: Int
    private let capacity: Int
    private let prime: Int
    private let samples: UnsafeMutablePointer<Float>
    private let written = Atomic<Int>(0)
    private let read = Atomic<Int>(0)
    // Used only on the render thread: whether enough sound waited to play
    private var playing = false

    init(channels: Int, capacity: Int, prime: Int) {
        self.channels = channels
        self.capacity = capacity
        self.prime = prime
        samples = .allocate(capacity: capacity * channels)
    }

    deinit {
        samples.deallocate()
    }

    /// Frames waiting to be played
    var buffered: Int {
        written.load(ordering: .acquiring) - read.load(ordering: .acquiring)
    }

    /// Adds interleaved frames, unless they do not fit.
    func write(_ pcm: UnsafePointer<Float>, frames: Int) {
        let start = written.load(ordering: .relaxed)

        guard start + frames - read.load(ordering: .acquiring) <= capacity else {
            return
        }

        for frame in 0..<frames {
            let slot = (start + frame) % capacity * channels
            for channel in 0..<channels {
                samples[slot + channel] = pcm[frame * channels + channel]
            }
        }

        written.store(start + frames, ordering: .releasing)
    }

    /// Fills the device's buffers, one per channel, and returns whether they hold only silence.
    func render(_ list: UnsafeMutableAudioBufferListPointer, frames: Int) -> Bool {
        let start = read.load(ordering: .relaxed)
        let available = written.load(ordering: .acquiring) - start

        if !playing && available >= prime {
            playing = true
        }

        let count = playing ? min(available, frames) : 0

        if playing && available < frames {
            playing = false
            underruns.add(1, ordering: .relaxed)
        }

        for (channel, buffer) in list.enumerated() {
            let output = buffer.mData!.assumingMemoryBound(to: Float.self)

            for frame in 0..<count {
                output[frame] = samples[(start + frame) % capacity * channels + channel]
            }

            for frame in count..<frames {
                output[frame] = 0
            }
        }

        read.store(start + count, ordering: .releasing)
        return count == 0
    }
}

// One access unit for the converter, handed over once
private struct Input {
    var bytes: UnsafeRawPointer?
    var packet: AudioStreamPacketDescription
}

// Returned by `supply` once its access unit was taken, which ends the conversion with what was decoded.
private let drained: OSStatus = 1

private let supply: AudioConverterComplexInputDataProc = { _, packets, data, descriptions, user in
    let input = user!.assumingMemoryBound(to: Input.self)

    guard let bytes = input.pointee.bytes else {
        packets.pointee = 0
        return drained
    }

    data.pointee.mNumberBuffers = 1
    data.pointee.mBuffers.mData = UnsafeMutableRawPointer(mutating: bytes)
    data.pointee.mBuffers.mDataByteSize = input.pointee.packet.mDataByteSize
    descriptions?.pointee = input.pointer(to: \.packet)
    packets.pointee = 1
    input.pointee.bytes = nil
    return noErr
}

private let log = Logger(subsystem: Diagnostics.subsystem, category: "Audio")
