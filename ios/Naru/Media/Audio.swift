import AudioToolbox
import AVFoundation
import os

/// Plays the Mac's audio by feeding AAC-ELD to an AudioQueue, which decodes it with the system decoder.
///
/// - Call only from the session's event thread. Audio plays as it arrives, without syncing to video.
/// - At most `maxQueued` frames wait to play. Frames beyond that are dropped so latency cannot build up, as when the
///   queue stalls in the background and resumes with everything sent meanwhile.
/// - The AudioSpecificConfig is wrapped in an MPEG-4 ES descriptor, the magic cookie format AudioToolbox reads, so the system determines the sample rate and channels.
final class Audio {
    // Frames waiting to play, 10 ms each
    private static let maxQueued = 6

    private let queue: AudioQueueRef
    private let queued: OSAllocatedUnfairLock<Int>

    init?(config: Data) {
        let cookie = Self.cookie(config)
        var format = AudioStreamBasicDescription()
        format.mFormatID = kAudioFormatMPEG4AAC_ELD_SBR
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)

        let described = cookie.withUnsafeBytes {
            AudioFormatGetProperty(kAudioFormatProperty_FormatInfo, UInt32(cookie.count), $0.baseAddress, &size, &format)
        }

        guard described == noErr else {
            return nil
        }

        let queued = OSAllocatedUnfairLock(initialState: 0)
        var queue: AudioQueueRef?
        let created = AudioQueueNewOutputWithDispatchQueue(&queue, &format, 0, DispatchQueue(label: "audio")) { queue, buffer in
            AudioQueueFreeBuffer(queue, buffer)
            queued.withLock { $0 -= 1 }
        }

        guard created == noErr, let queue else {
            return nil
        }

        self.queue = queue
        self.queued = queued
        _ = cookie.withUnsafeBytes { AudioQueueSetProperty(queue, kAudioQueueProperty_MagicCookie, $0.baseAddress!, UInt32(cookie.count)) }

        try? AVAudioSession.sharedInstance().setCategory(.playback, options: .mixWithOthers)
        try? AVAudioSession.sharedInstance().setActive(true)
        AudioQueueStart(queue, nil)
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
        guard queued.withLock({ $0 < Self.maxQueued }) else {
            return
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
            queued.withLock { $0 += 1 }
        } else {
            AudioQueueFreeBuffer(queue, buffer)
        }
    }
}
