import AudioToolbox
import AVFoundation
import os

/// Plays the Mac's audio by feeding AAC-ELD to an AudioQueue, which decodes it with the system decoder.
///
/// - Call only from the session's event thread. Frames come paced by the session, which keeps them with the picture.
/// - At most `maxBacklog` of sound waits to be heard, by the queue's own clock, and frames beyond it are dropped. The
///   queue takes frames before it plays them, so counting frames in it would let a burst of sound pile up for good, as
///   when the session catches up after the network paused; the picture skips ahead then, and so does the sound.
/// - A queue whose clock stops while frames keep coming is replaced, as one did for good after the network stalled.
/// - The AudioSpecificConfig is wrapped in an MPEG-4 ES descriptor, the magic cookie format AudioToolbox reads, so the system determines the sample rate and channels.
final class Audio {
    // Each AAC-ELD frame carries 480 samples at 48 kHz, 10 ms.
    private static let frameSamples = 480.0
    private static let maxBacklog = frameSamples * 6
    // A queue whose clock has not moved for this long has stopped. Replacing one is tried no more often than this.
    private static let stalled = Duration.milliseconds(500)

    private var format: AudioStreamBasicDescription
    private let cookie: Data
    private var queue: AudioQueueRef
    // Samples handed to the queue, on the queue's clock, and when that clock last moved
    private var enqueued = 0.0
    private var played = 0.0
    private var moved = ContinuousClock.now
    // Diagnostics: the last frame's timestamp, and how many frames in a row were dropped for a backlog
    private var lastTimestamp: UInt32?
    private var dropped = 0

    init?(config: Data) {
        let cookie = Self.cookie(config)
        var format = AudioStreamBasicDescription()
        format.mFormatID = kAudioFormatMPEG4AAC_ELD_SBR
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)

        let described = cookie.withUnsafeBytes {
            AudioFormatGetProperty(kAudioFormatProperty_FormatInfo, UInt32(cookie.count), $0.baseAddress, &size, &format)
        }

        guard described == noErr, let queue = Self.start(&format, cookie: cookie) else {
            return nil
        }

        self.format = format
        self.cookie = cookie
        self.queue = queue
        try? AVAudioSession.sharedInstance().setCategory(.playback, options: .mixWithOthers)
        try? AVAudioSession.sharedInstance().setActive(true)
    }

    private static func start(_ format: inout AudioStreamBasicDescription, cookie: Data) -> AudioQueueRef? {
        var queue: AudioQueueRef?
        let created = AudioQueueNewOutputWithDispatchQueue(&queue, &format, 0, DispatchQueue(label: "audio")) { queue, buffer in
            AudioQueueFreeBuffer(queue, buffer)
        }

        guard created == noErr, let queue else {
            return nil
        }

        _ = cookie.withUnsafeBytes { AudioQueueSetProperty(queue, kAudioQueueProperty_MagicCookie, $0.baseAddress!, UInt32(cookie.count)) }
        AudioQueueStart(queue, nil)
        return queue
    }

    // ES descriptor > decoder config descriptor (MPEG-4 audio, audio stream) > decoder specific info, then the SL config descriptor.
    // AudioToolbox rejects a bare AudioSpecificConfig with kAudioFormatUnsupportedDataFormatError.
    private static func cookie(_ config: Data) -> Data {
        func descriptor(_ tag: UInt8, _ body: Data) -> Data { Data([tag, UInt8(body.count)]) + body }

        let decoderConfig = descriptor(0x04, Data([0x40, 0x15] + [UInt8](repeating: 0, count: 11)) + descriptor(0x05, config))
        return descriptor(0x03, Data([0, 0, 0]) + decoderConfig + descriptor(0x06, Data([0x02])))
    }

    deinit {
        AudioQueueDispose(queue, true)
    }

    func play(_ frame: AudioFrame) {
        let now = ContinuousClock.now

        // Each frame is 480 samples at 48 kHz, so any other step means frames the network lost.
        if let lastTimestamp, frame.timestamp &- lastTimestamp != 480 {
            log.log("audio stream skipped from \(lastTimestamp) to \(frame.timestamp)")
        }
        lastTimestamp = frame.timestamp

        var time = AudioTimeStamp()
        if AudioQueueGetCurrentTime(queue, nil, &time, nil) == noErr, time.mSampleTime > played {
            played = time.mSampleTime
            moved = now
        }

        // A queue that ran dry plays the next frame at once, so what it had is no longer waiting.
        enqueued = max(enqueued, played)

        if enqueued - played >= Self.maxBacklog {
            guard now - moved >= Self.stalled else {
                dropped += 1
                return
            }

            moved = now

            guard let fresh = Self.start(&format, cookie: cookie) else {
                dropped += 1
                return
            }

            log.error("audio queue stopped playing with \(Int(self.enqueued - self.played)) samples waiting; starting a new one")
            AudioQueueDispose(queue, true)
            queue = fresh
            enqueued = 0
            played = 0
        }

        if dropped > 0 {
            log.log("audio dropped \(self.dropped) frames to keep up")
            dropped = 0
        }

        let data = frame.data
        var buffer: AudioQueueBufferRef?

        guard AudioQueueAllocateBufferWithPacketDescriptions(queue, UInt32(data.count), 1, &buffer) == noErr, let buffer else {
            return
        }

        data.withUnsafeBytes { buffer.pointee.mAudioData.copyMemory(from: $0.baseAddress!, byteCount: data.count) }
        buffer.pointee.mAudioDataByteSize = UInt32(data.count)

        var packet = AudioStreamPacketDescription(mStartOffset: 0, mVariableFramesInPacket: 0, mDataByteSize: UInt32(data.count))
        if AudioQueueEnqueueBuffer(queue, buffer, 1, &packet) == noErr {
            enqueued += Self.frameSamples
        } else {
            AudioQueueFreeBuffer(queue, buffer)
        }
    }
}

private let log = Logger(subsystem: Diagnostics.subsystem, category: "Audio")
